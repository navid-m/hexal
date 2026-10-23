using System.ComponentModel;
using System.Diagnostics;

namespace Hexal.Core;

public sealed class FormatterConfig
{
    public string Command { get; init; } = string.Empty;

    public IReadOnlyList<string> Arguments { get; init; } = Array.Empty<string>();

    public string Input { get; init; } = "stdin";

    public string Output { get; init; } = "stdout";

    public int TimeoutMs { get; init; } = 15000;

    public bool UsesStdin => !string.Equals(Input, "file", StringComparison.OrdinalIgnoreCase);

    public bool WritesFile => string.Equals(Output, "file", StringComparison.OrdinalIgnoreCase);
}

public sealed class FormatResult
{
    public bool Ok { get; init; }

    public byte[] Output { get; init; } = Array.Empty<byte>();

    public string Error { get; init; } = string.Empty;

    public static FormatResult Failure(string error) => new() { Ok = false, Error = error };

    public static FormatResult Success(byte[] output) => new() { Ok = true, Output = output };
}

public static class FormatterRunner
{
    private const int DefaultTimeoutMs = 15000;

    public static FormatResult Run(
        FormatterConfig config,
        byte[] input,
        string? documentPath,
        CancellationToken cancellationToken
    )
    {
        if (string.IsNullOrWhiteSpace(config.Command))
        {
            return FormatResult.Failure("no formatter command configured");
        }

        bool usesStdin = config.UsesStdin;
        bool writesFile = config.WritesFile;
        string? tempPath = null;

        try
        {
            if (!usesStdin || writesFile)
            {
                tempPath = CreateTempPath(documentPath);
            }

            if (!usesStdin)
            {
                File.WriteAllBytes(tempPath!, input);
            }

            var startInfo = new ProcessStartInfo
            {
                FileName = config.Command,
                UseShellExecute = false,
                RedirectStandardInput = usesStdin,
                RedirectStandardOutput = !writesFile,
                RedirectStandardError = true,
                CreateNoWindow = true,
            };

            string? workingDirectory = WorkingDirectory(documentPath);
            if (workingDirectory is not null)
            {
                startInfo.WorkingDirectory = workingDirectory;
            }

            foreach (string argument in config.Arguments)
            {
                startInfo.ArgumentList.Add(Substitute(argument, documentPath, tempPath));
            }

            using var process = new Process { StartInfo = startInfo };
            try
            {
                if (!process.Start())
                {
                    return FormatResult.Failure($"cannot start formatter '{config.Command}'");
                }
            }
            catch (Win32Exception ex)
            {
                return FormatResult.Failure(
                    $"cannot start formatter '{config.Command}': {ex.Message}"
                );
            }
            catch (InvalidOperationException ex)
            {
                return FormatResult.Failure(
                    $"cannot start formatter '{config.Command}': {ex.Message}"
                );
            }

            Task<byte[]>? stdoutTask = writesFile
                ? null
                : ReadAllAsync(process.StandardOutput.BaseStream);
            Task<string> stderrTask = process.StandardError.ReadToEndAsync(CancellationToken.None);
            Task? writeTask = usesStdin
                ? WriteInputAsync(process.StandardInput.BaseStream, input)
                : null;

            int timeoutMs = config.TimeoutMs > 0 ? config.TimeoutMs : DefaultTimeoutMs;

            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timeout.CancelAfter(timeoutMs);

            try
            {
                process.WaitForExitAsync(timeout.Token).GetAwaiter().GetResult();
            }
            catch (OperationCanceledException)
            {
                TryKill(process);
                string reason = cancellationToken.IsCancellationRequested
                    ? "formatting cancelled"
                    : $"formatter timed out after {timeoutMs} ms";
                return FormatResult.Failure(reason);
            }

            try
            {
                writeTask?.GetAwaiter().GetResult();
            }
            catch (IOException) { }
            catch (ObjectDisposedException) { }

            string stderr = stderrTask.GetAwaiter().GetResult();
            byte[] stdout = stdoutTask is null
                ? Array.Empty<byte>()
                : stdoutTask.GetAwaiter().GetResult();

            if (process.ExitCode != 0)
            {
                string detail = stderr.Trim();
                return FormatResult.Failure(
                    detail.Length > 0
                        ? $"formatter exited with code {process.ExitCode}: {detail}"
                        : $"formatter exited with code {process.ExitCode}"
                );
            }

            byte[] output;
            if (writesFile)
            {
                if (tempPath is null || !File.Exists(tempPath))
                {
                    return FormatResult.Failure("formatter did not produce an output file");
                }

                output = File.ReadAllBytes(tempPath);
            }
            else
            {
                output = stdout;
            }

            if (output.Length >= 3 && output[0] == 0xEF && output[1] == 0xBB && output[2] == 0xBF)
            {
                output = output[3..];
            }

            return FormatResult.Success(output);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            return FormatResult.Failure($"formatter failed: {ex.Message}");
        }
        finally
        {
            if (tempPath is not null)
            {
                try
                {
                    File.Delete(tempPath);
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { }
            }
        }
    }

    private static string CreateTempPath(string? documentPath)
    {
        string extension = string.Empty;
        if (!string.IsNullOrEmpty(documentPath))
        {
            try
            {
                extension = Path.GetExtension(documentPath);
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException)
            {
                extension = string.Empty;
            }
        }

        if (extension.Length == 0)
        {
            extension = ".tmp";
        }

        return Path.Combine(Path.GetTempPath(), $"hexal-format-{Guid.NewGuid():N}{extension}");
    }

    private static string? WorkingDirectory(string? documentPath)
    {
        if (string.IsNullOrEmpty(documentPath))
        {
            return null;
        }

        try
        {
            return Path.GetDirectoryName(Path.GetFullPath(documentPath));
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return null;
        }
    }

    private static string Substitute(string argument, string? documentPath, string? tempPath)
    {
        string file = string.IsNullOrEmpty(documentPath) ? tempPath ?? string.Empty : documentPath;
        string temp = tempPath ?? file;

        return argument
            .Replace("{file}", file, StringComparison.Ordinal)
            .Replace("{path}", file, StringComparison.Ordinal)
            .Replace("{tmp}", temp, StringComparison.Ordinal)
            .Replace("{temp}", temp, StringComparison.Ordinal)
            .Replace("{output}", temp, StringComparison.Ordinal);
    }

    private static async Task<byte[]> ReadAllAsync(Stream stream)
    {
        using var buffer = new MemoryStream();
        await stream.CopyToAsync(buffer).ConfigureAwait(false);
        return buffer.ToArray();
    }

    private static async Task WriteInputAsync(Stream stream, byte[] input)
    {
        try
        {
            await stream.WriteAsync(input).ConfigureAwait(false);
            await stream.FlushAsync().ConfigureAwait(false);
        }
        catch (IOException) { }
        finally
        {
            try
            {
                stream.Close();
            }
            catch (IOException) { }
            catch (ObjectDisposedException) { }
        }
    }

    private static void TryKill(Process process)
    {
        try
        {
            if (!process.HasExited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
        catch (InvalidOperationException) { }
        catch (Win32Exception) { }
        catch (NotSupportedException) { }
    }
}

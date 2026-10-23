using System.ComponentModel;
using System.Diagnostics;
using System.Text;

namespace Hexal.Platform;

/// <summary>
/// Reads and writes the operating system clipboard. Native clipboard helpers
/// are preferred so the clipboard works independently of terminal support;
/// copying falls back to the OSC 52 escape sequence, which also works over
/// SSH when the terminal implements it.
/// </summary>
public static class Clipboard
{
    private const int HelperTimeoutMilliseconds = 5000;

    private const string WindowsCopyCommand =
        "[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false); "
        + "$text = [Console]::In.ReadToEnd(); Set-Clipboard -Value $text";

    private const string WindowsPasteCommand =
        "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); "
        + "$text = Get-Clipboard -Raw; if ($null -ne $text) { [Console]::Out.Write($text) }";

    /// <summary>
    /// Copies UTF-8 text to the system clipboard. Returns false only when the
    /// text is empty; a missing native helper is not an error because the
    /// terminal may still handle OSC 52.
    /// </summary>
    public static bool TryCopy(ITerminal terminal, ReadOnlySpan<byte> text, out string error)
    {
        error = string.Empty;
        if (text.IsEmpty)
        {
            error = "nothing selected";
            return false;
        }

        if (OperatingSystem.IsWindows() && WindowsClipboard.TrySetText(text, out _))
        {
            return true;
        }

        foreach ((string program, string[] arguments) in CopyCommands())
        {
            if (TryRunWriter(program, arguments, text, out _))
            {
                return true;
            }
        }

        // OSC 52 asks the terminal to place the base64 payload on the
        // clipboard. Unlike the helpers this also reaches the local machine
        // when the editor runs inside an SSH session.
        byte[] sequence = BuildOsc52(text);
        terminal.Write(sequence);
        return true;
    }

    /// <summary>
    /// Reads the system clipboard as raw UTF-8 bytes.
    /// </summary>
    public static bool TryPaste(out byte[] text, out string error)
    {
        text = Array.Empty<byte>();

        string lastError = string.Empty;
        if (OperatingSystem.IsWindows())
        {
            if (WindowsClipboard.TryGetText(out text, out string nativeError))
            {
                error = string.Empty;
                return true;
            }

            if (nativeError.Length > 0)
            {
                lastError = nativeError;
            }
        }

        foreach ((string program, string[] arguments) in PasteCommands())
        {
            if (TryRunReader(program, arguments, out text, out string candidateError))
            {
                error = string.Empty;
                return true;
            }

            if (candidateError.Length > 0)
            {
                lastError = candidateError;
            }
        }

        error =
            lastError.Length > 0
                ? $"cannot read the system clipboard: {lastError}"
                : "cannot read the system clipboard: no clipboard helper found";
        return false;
    }

    private static IEnumerable<(string Program, string[] Arguments)> CopyCommands()
    {
        if (OperatingSystem.IsWindows())
        {
            yield return (
                "pwsh.exe",
                new[] { "-NoProfile", "-NonInteractive", "-Command", WindowsCopyCommand }
            );
            yield return (
                "powershell.exe",
                new[] { "-NoProfile", "-NonInteractive", "-Command", WindowsCopyCommand }
            );
            yield break;
        }

        if (OperatingSystem.IsMacOS())
        {
            yield return ("pbcopy", Array.Empty<string>());
            yield break;
        }

        if (!string.IsNullOrEmpty(Environment.GetEnvironmentVariable("WAYLAND_DISPLAY")))
        {
            yield return ("wl-copy", Array.Empty<string>());
        }

        yield return ("xclip", new[] { "-selection", "clipboard", "-in" });
        yield return ("xsel", new[] { "--clipboard", "--input" });
    }

    private static IEnumerable<(string Program, string[] Arguments)> PasteCommands()
    {
        if (OperatingSystem.IsWindows())
        {
            yield return (
                "pwsh.exe",
                new[] { "-NoProfile", "-NonInteractive", "-Command", WindowsPasteCommand }
            );
            yield return (
                "powershell.exe",
                new[] { "-NoProfile", "-NonInteractive", "-Command", WindowsPasteCommand }
            );
            yield break;
        }

        if (OperatingSystem.IsMacOS())
        {
            yield return ("pbpaste", Array.Empty<string>());
            yield break;
        }

        if (!string.IsNullOrEmpty(Environment.GetEnvironmentVariable("WAYLAND_DISPLAY")))
        {
            yield return ("wl-paste", new[] { "--no-newline" });
        }

        yield return ("xclip", new[] { "-selection", "clipboard", "-out" });
        yield return ("xsel", new[] { "--clipboard", "--output" });
    }

    private static byte[] BuildOsc52(ReadOnlySpan<byte> text)
    {
        string payload = Convert.ToBase64String(text);
        return Encoding.ASCII.GetBytes($"\u001b]52;c;{payload}\u0007");
    }

    private static bool TryRunWriter(
        string program,
        string[] arguments,
        ReadOnlySpan<byte> text,
        out string error
    )
    {
        error = string.Empty;

        try
        {
            using Process? process = Process.Start(
                CreateStartInfo(program, arguments, redirectOutput: false)
            );
            if (process is null)
            {
                error = $"{program} did not start";
                return false;
            }

            try
            {
                process.StandardInput.BaseStream.Write(text);
                process.StandardInput.Close();
            }
            catch (IOException)
            {
                // A helper that rejects its input reports the reason on stderr.
            }

            if (!process.WaitForExit(HelperTimeoutMilliseconds))
            {
                // Some X11 helpers fork a background process that keeps
                // ownership of the selection. Reaching the timeout therefore
                // does not mean the copy failed.
                return true;
            }

            if (process.ExitCode == 0)
            {
                return true;
            }

            string stderr = process.StandardError.ReadToEnd().Trim();
            error = stderr.Length > 0 ? stderr : $"{program} exited with code {process.ExitCode}";
            return false;
        }
        catch (Exception ex) when (IsHelperFailure(ex))
        {
            // Win32Exception is the normal "helper is not installed" path.
            error = ex.Message;
            return false;
        }
    }

    private static bool TryRunReader(
        string program,
        string[] arguments,
        out byte[] output,
        out string error
    )
    {
        output = Array.Empty<byte>();
        error = string.Empty;

        try
        {
            using Process? process = Process.Start(
                CreateStartInfo(program, arguments, redirectOutput: true)
            );
            if (process is null)
            {
                error = $"{program} did not start";
                return false;
            }

            process.StandardInput.Close();
            Task<byte[]> stdoutTask = ReadAllBytesAsync(process.StandardOutput.BaseStream);

            if (!process.WaitForExit(HelperTimeoutMilliseconds))
            {
                try
                {
                    process.Kill(entireProcessTree: true);
                }
                catch (Exception ex) when (IsHelperFailure(ex))
                {
                    // The process may already have exited.
                }

                error = $"{program} timed out";
                return false;
            }

            output = stdoutTask.GetAwaiter().GetResult();
            if (process.ExitCode == 0)
            {
                return true;
            }

            string stderr = process.StandardError.ReadToEnd().Trim();
            error = stderr.Length > 0 ? stderr : $"{program} exited with code {process.ExitCode}";
            return false;
        }
        catch (Exception ex) when (IsHelperFailure(ex))
        {
            error = ex.Message;
            return false;
        }
    }

    private static ProcessStartInfo CreateStartInfo(
        string program,
        string[] arguments,
        bool redirectOutput
    )
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = program,
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardInput = true,
            RedirectStandardOutput = redirectOutput,
            RedirectStandardError = true,
            StandardOutputEncoding = redirectOutput ? Encoding.UTF8 : null,
            StandardErrorEncoding = Encoding.UTF8,
        };

        foreach (string argument in arguments)
        {
            startInfo.ArgumentList.Add(argument);
        }

        return startInfo;
    }

    private static async Task<byte[]> ReadAllBytesAsync(Stream stream)
    {
        using var buffer = new MemoryStream();
        await stream.CopyToAsync(buffer).ConfigureAwait(false);
        return buffer.ToArray();
    }

    private static bool IsHelperFailure(Exception ex) =>
        ex
            is Win32Exception
                or IOException
                or InvalidOperationException
                or UnauthorizedAccessException
                or NotSupportedException
                or ObjectDisposedException;
}

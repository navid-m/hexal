using System.ComponentModel;
using System.Diagnostics;

namespace Hexal.Scripting;

/// <summary>
/// Installs a plugin folder from the configured central git repository.
/// Plugin directories live at the repository root.
/// </summary>
public static class PluginInstaller
{
    private const int CloneTimeoutMs = 120_000;

    public static bool TryInstall(string name, string repositoryUrl,
        string pluginDirectory, out string installedPath, out string error)
    {
        installedPath = string.Empty;
        error = string.Empty;

        if (!IsValidPluginName(name))
        {
            error = "invalid plugin name";
            return false;
        }

        if (string.IsNullOrWhiteSpace(repositoryUrl))
        {
            error = "central repository URL is empty";
            return false;
        }

        string destination;
        try
        {
            Directory.CreateDirectory(pluginDirectory);
            destination = Path.Combine(Path.GetFullPath(pluginDirectory), name);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   PathTooLongException)
        {
            error = ex.Message;
            return false;
        }

        if (File.Exists(destination) || Directory.Exists(destination))
        {
            error = $"plugin '{name}' is already installed at '{destination}'";
            return false;
        }

        string temporaryRoot = Path.Combine(Path.GetTempPath(),
            "hexal-plugin-" + Guid.NewGuid().ToString("N"));
        try
        {
            Directory.CreateDirectory(temporaryRoot);
            string cloneDirectory = Path.Combine(temporaryRoot, "repository");
            if (!CloneRepository(repositoryUrl, cloneDirectory, out error))
            {
                return false;
            }

            string source = Path.Combine(cloneDirectory, name);
            if (!Directory.Exists(source))
            {
                error = $"plugin '{name}' was not found in '{repositoryUrl}'";
                return false;
            }

            string staging = destination + ".installing-" +
                             Guid.NewGuid().ToString("N");
            try
            {
                CopyDirectory(source, staging);
                Directory.Move(staging, destination);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException or
                                       PathTooLongException)
            {
                TryDeleteDirectory(staging);
                error = ex.Message;
                return false;
            }

            installedPath = destination;
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   PathTooLongException)
        {
            error = ex.Message;
            return false;
        }
        finally
        {
            TryDeleteDirectory(temporaryRoot);
        }
    }

    private static bool CloneRepository(string repositoryUrl, string directory,
        out string error)
    {
        error = string.Empty;
        var startInfo = new ProcessStartInfo
        {
            FileName = "git",
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true
        };
        startInfo.ArgumentList.Add("clone");
        startInfo.ArgumentList.Add("--depth");
        startInfo.ArgumentList.Add("1");
        startInfo.ArgumentList.Add("--quiet");
        startInfo.ArgumentList.Add("--");
        startInfo.ArgumentList.Add(repositoryUrl);
        startInfo.ArgumentList.Add(directory);

        try
        {
            using var process = new Process { StartInfo = startInfo };
            if (!process.Start())
            {
                error = "cannot start git";
                return false;
            }

            Task<string> stdout = process.StandardOutput.ReadToEndAsync();
            Task<string> stderr = process.StandardError.ReadToEndAsync();
            using var cancellation = new CancellationTokenSource(CloneTimeoutMs);
            try
            {
                process.WaitForExitAsync(cancellation.Token)
                    .GetAwaiter().GetResult();
            }
            catch (OperationCanceledException)
            {
                try
                {
                    if (!process.HasExited)
                    {
                        process.Kill(entireProcessTree: true);
                    }
                }
                catch
                {
                    // Process teardown is best effort.
                }

                error = $"git clone timed out after {CloneTimeoutMs} ms";
                return false;
            }

            string output = stdout.GetAwaiter().GetResult().Trim();
            string detail = stderr.GetAwaiter().GetResult().Trim();
            if (process.ExitCode == 0)
            {
                return true;
            }

            error = detail.Length > 0
                ? detail
                : output.Length > 0
                    ? output
                    : $"git clone exited with code {process.ExitCode}";
            return false;
        }
        catch (Exception ex) when (ex is Win32Exception or InvalidOperationException or
                                   IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            error = $"cannot run git: {ex.Message}";
            return false;
        }
    }

    private static bool IsValidPluginName(string name)
    {
        if (string.IsNullOrWhiteSpace(name) || name is "." or "..")
        {
            return false;
        }

        if (name.Contains(Path.DirectorySeparatorChar) ||
            name.Contains(Path.AltDirectorySeparatorChar))
        {
            return false;
        }

        return name.IndexOfAny(Path.GetInvalidFileNameChars()) < 0;
    }

    private static void CopyDirectory(string source, string destination)
    {
        Directory.CreateDirectory(destination);

        foreach (string file in Directory.GetFiles(source))
        {
            File.Copy(file, Path.Combine(destination, Path.GetFileName(file)),
                overwrite: false);
        }

        foreach (string directory in Directory.GetDirectories(source))
        {
            FileAttributes attributes = File.GetAttributes(directory);
            if ((attributes & FileAttributes.ReparsePoint) != 0)
            {
                continue;
            }

            CopyDirectory(directory,
                Path.Combine(destination, Path.GetFileName(directory)));
        }
    }

    private static void TryDeleteDirectory(string path)
    {
        try
        {
            if (Directory.Exists(path))
            {
                Directory.Delete(path, recursive: true);
            }
        }
        catch
        {
            // Best-effort cleanup only.
        }
    }
}

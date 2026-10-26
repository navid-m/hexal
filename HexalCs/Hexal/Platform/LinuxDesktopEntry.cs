using System.Diagnostics;
using System.Text;
using Hexal.Util;

namespace Hexal.Platform;

/// <summary>
/// Installs a Linux desktop launcher so file managers can open Hexal through
/// the normal application association mechanism. The launcher is written to
/// <c>$XDG_DATA_HOME/applications</c> (usually
/// <c>~/.local/share/applications</c>) and falls back to the executable's
/// directory when that location cannot be written. It uses
/// <c>Terminal=true</c>, so desktop environments start Hexal in a terminal
/// instead of trying to run the TUI directly.
/// </summary>
internal static class LinuxDesktopEntry
{
    private const string DesktopFileName = "hexal.desktop";
    private const UnixFileMode DesktopFileMode =
        UnixFileMode.UserRead
        | UnixFileMode.UserWrite
        | UnixFileMode.UserExecute
        | UnixFileMode.GroupRead
        | UnixFileMode.GroupExecute
        | UnixFileMode.OtherRead
        | UnixFileMode.OtherExecute;

    private static bool _checked;

    public static void EnsureLauncher()
    {
        if (_checked || !OperatingSystem.IsLinux())
        {
            return;
        }

        _checked = true;
        long launcherStarted = StartupTrace.Timestamp();

        try
        {
            long phaseStarted = StartupTrace.Timestamp();
            string? executable = Environment.ProcessPath;
            StartupTrace.Mark(
                "launcher: resolve process path",
                phaseStarted,
                executable ?? "<null>"
            );
            if (string.IsNullOrWhiteSpace(executable))
            {
                LogFailure("could not resolve the executable path");
                return;
            }

            string executableName = Path.GetFileName(executable);
            if (
                string.Equals(executableName, "dotnet", StringComparison.OrdinalIgnoreCase)
                || string.Equals(executableName, "dotnet.exe", StringComparison.OrdinalIgnoreCase)
            )
            {
                StartupTrace.Note("launcher: managed dotnet host; skipping");
                return;
            }

            phaseStarted = StartupTrace.Timestamp();
            executable = Path.GetFullPath(executable);
            string? executableDirectory = Path.GetDirectoryName(executable);
            StartupTrace.Mark("launcher: resolve executable path", phaseStarted, executable);
            if (string.IsNullOrEmpty(executableDirectory))
            {
                LogFailure($"could not resolve the directory for '{executable}'");
                return;
            }

            if (executable.AsSpan().ContainsAny('\r', '\n'))
            {
                LogFailure("the executable path contains a line break");
                return;
            }

            phaseStarted = StartupTrace.Timestamp();
            string contents = BuildContents(executable);
            StartupTrace.Mark(
                "launcher: build desktop contents",
                phaseStarted,
                $"{contents.Length} char(s)"
            );

            phaseStarted = StartupTrace.Timestamp();
            string? applicationsDirectory = ApplicationsDirectory();
            StartupTrace.Mark(
                "launcher: resolve applications directory",
                phaseStarted,
                applicationsDirectory ?? "<null>"
            );

            if (applicationsDirectory is not null)
            {
                string applicationsPath = Path.Combine(
                    applicationsDirectory,
                    DesktopFileName
                );
                phaseStarted = StartupTrace.Timestamp();
                bool installed = TryInstallLauncher(
                    applicationsPath,
                    contents,
                    createDirectory: true,
                    out bool changed,
                    out string error
                );
                StartupTrace.Mark(
                    "launcher: install in XDG applications",
                    phaseStarted,
                    installed ? $"changed={changed}" : error
                );
                if (installed)
                {
                    if (changed)
                    {
                        HexalPaths.Log($"Installed desktop launcher: {applicationsPath}");
                        TryUpdateDesktopDatabase(applicationsDirectory);
                    }
                    return;
                }

                HexalPaths.Log(
                    $"Could not install desktop launcher in '{applicationsDirectory}': "
                        + $"{error}; falling back to the executable directory"
                );
            }

            string fallbackPath = Path.Combine(executableDirectory, DesktopFileName);
            phaseStarted = StartupTrace.Timestamp();
            if (
                !TryInstallLauncher(
                    fallbackPath,
                    contents,
                    createDirectory: false,
                    out bool fallbackChanged,
                    out string fallbackError
                )
            )
            {
                StartupTrace.Mark(
                    "launcher: install fallback",
                    phaseStarted,
                    fallbackError
                );
                LogFailure(fallbackError);
                return;
            }
            StartupTrace.Mark(
                "launcher: install fallback",
                phaseStarted,
                $"changed={fallbackChanged}"
            );

            if (fallbackChanged)
            {
                HexalPaths.Log($"Created desktop launcher: {fallbackPath}");
            }
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
                        or PathTooLongException
                        or System.Security.SecurityException
            )
        {
            LogFailure(ex.Message);
        }
        finally
        {
            StartupTrace.Mark("launcher: total", launcherStarted);
        }
    }

    private static string? ApplicationsDirectory()
    {
        string? dataHome = Environment.GetEnvironmentVariable("XDG_DATA_HOME");
        if (string.IsNullOrWhiteSpace(dataHome) || !Path.IsPathRooted(dataHome))
        {
            string home = HexalPaths.HomeDirectory;
            if (string.IsNullOrEmpty(home))
            {
                return null;
            }

            dataHome = Path.Combine(home, ".local", "share");
        }

        return Path.Combine(dataHome, "applications");
    }

    private static bool TryInstallLauncher(
        string path,
        string contents,
        bool createDirectory,
        out bool changed,
        out string error
    )
    {
        changed = false;
        error = string.Empty;

        try
        {
            long phaseStarted = StartupTrace.Timestamp();
            string? directory = Path.GetDirectoryName(path);
            if (createDirectory && !string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }
            StartupTrace.Mark("launcher: ensure applications directory", phaseStarted, path);

            if (File.Exists(path))
            {
                phaseStarted = StartupTrace.Timestamp();
                try
                {
                    if (
                        string.Equals(
                            File.ReadAllText(path),
                            contents,
                            StringComparison.Ordinal
                        )
                    )
                    {
                        StartupTrace.Mark("launcher: read/compare existing", phaseStarted, "unchanged");
                        return true;
                    }
                }
                catch (Exception ex)
                    when (ex
                            is IOException
                                or UnauthorizedAccessException
                                or NotSupportedException
                    )
                {
                    // Replace the existing launcher if it cannot be read.
                    StartupTrace.Mark(
                        "launcher: read/compare existing",
                        phaseStarted,
                        $"read failed: {ex.Message}"
                    );
                }
                StartupTrace.Mark("launcher: read/compare existing", phaseStarted, "changed");
            }
            else
            {
                StartupTrace.Note($"launcher: no existing file at '{path}'");
            }

            phaseStarted = StartupTrace.Timestamp();
            File.WriteAllText(path, contents, new UTF8Encoding(false));
            changed = true;
            StartupTrace.Mark("launcher: write file", phaseStarted, $"{contents.Length} char(s)");

            if (OperatingSystem.IsLinux())
            {
                phaseStarted = StartupTrace.Timestamp();
                try
                {
                    File.SetUnixFileMode(path, DesktopFileMode);
                }
                catch (Exception ex)
                    when (ex
                            is IOException
                                or UnauthorizedAccessException
                                or NotSupportedException
                                or ArgumentException
                    )
                {
                    // The file is still usable without the executable bit.
                    StartupTrace.Mark("launcher: set file mode", phaseStarted, ex.Message);
                }
                StartupTrace.Mark("launcher: set file mode", phaseStarted);
            }

            return true;
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
                        or PathTooLongException
                        or System.Security.SecurityException
            )
        {
            error = ex.Message;
            return false;
        }
    }

    private static void TryUpdateDesktopDatabase(string applicationsDirectory)
    {
        long updateStarted = StartupTrace.Timestamp();
        try
        {
            var startInfo = new ProcessStartInfo
            {
                FileName = "update-desktop-database",
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            };
            startInfo.ArgumentList.Add(applicationsDirectory);

            long phaseStarted = StartupTrace.Timestamp();
            using Process? process = Process.Start(startInfo);
            StartupTrace.Mark("launcher: start update-desktop-database", phaseStarted);
            if (process is null)
            {
                return;
            }

            process.StandardInput.Close();
            _ = process.StandardOutput.ReadToEndAsync();
            _ = process.StandardError.ReadToEndAsync();
            phaseStarted = StartupTrace.Timestamp();
            bool exited = process.WaitForExit(2000);
            StartupTrace.Mark(
                "launcher: wait for update-desktop-database",
                phaseStarted,
                exited ? $"exit={process.ExitCode}" : "timed out after 2000 ms"
            );
            if (!exited)
            {
                process.Kill(entireProcessTree: true);
            }
        }
        catch (Exception ex)
            when (ex
                    is System.ComponentModel.Win32Exception
                        or IOException
                        or InvalidOperationException
                        or NotSupportedException
                        or UnauthorizedAccessException
            )
        {
            // The desktop database is an optional cache refresh.
            StartupTrace.Note($"launcher: update-desktop-database failed: {ex.Message}");
        }
        finally
        {
            StartupTrace.Mark("launcher: update-desktop-database total", updateStarted);
        }
    }

    internal static string BuildContents(string executable)
    {
        return $"""
            [Desktop Entry]
            Type=Application
            Version=1.0
            Name=Hexal
            GenericName=Text Editor
            Comment=Modal Text Editing Redefined
            Exec={QuoteExecArgument(executable)} %f
            Terminal=true
            Categories=Utility;TextEditor;
            MimeType=text/plain;
            Keywords=text;editor;terminal;modal;
            StartupNotify=false
            Icon=utilities-terminal

            """;
    }

    private static string QuoteExecArgument(string value)
    {
        var builder = new StringBuilder(value.Length + 8);
        builder.Append('"');
        foreach (char character in value)
        {
            switch (character)
            {
                case '\\':
                    builder.Append("\\\\");
                    break;
                case '"':
                    builder.Append("\\\"");
                    break;
                case '`':
                    builder.Append("\\`");
                    break;
                case '$':
                    builder.Append("\\$");
                    break;
                case '%':
                    builder.Append("%%");
                    break;
                default:
                    builder.Append(character);
                    break;
            }
        }
        builder.Append('"');
        return builder.ToString();
    }

    private static void LogFailure(string message) =>
        HexalPaths.Log($"Could not create desktop launcher: {message}");
}

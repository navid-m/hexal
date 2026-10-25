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

        try
        {
            string? executable = Environment.ProcessPath;
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
                return;
            }

            executable = Path.GetFullPath(executable);
            string? executableDirectory = Path.GetDirectoryName(executable);
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

            string contents = BuildContents(executable);
            string? applicationsDirectory = ApplicationsDirectory();
            if (applicationsDirectory is not null)
            {
                string applicationsPath = Path.Combine(
                    applicationsDirectory,
                    DesktopFileName
                );
                if (
                    TryInstallLauncher(
                        applicationsPath,
                        contents,
                        createDirectory: true,
                        out bool changed,
                        out string error
                    )
                )
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
                LogFailure(fallbackError);
                return;
            }

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
            string? directory = Path.GetDirectoryName(path);
            if (createDirectory && !string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }

            if (File.Exists(path))
            {
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
                }
            }

            File.WriteAllText(path, contents, new UTF8Encoding(false));
            changed = true;

            if (OperatingSystem.IsLinux())
            {
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
                }
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

            using Process? process = Process.Start(startInfo);
            if (process is null)
            {
                return;
            }

            process.StandardInput.Close();
            _ = process.StandardOutput.ReadToEndAsync();
            _ = process.StandardError.ReadToEndAsync();
            if (!process.WaitForExit(2000))
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

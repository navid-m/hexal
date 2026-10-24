using System.Text;
using Hexal.Util;

namespace Hexal.Platform;

/// <summary>
/// Creates a Linux desktop launcher beside the Hexal executable on first run.
/// The launcher uses <c>Terminal=true</c>, so desktop environments start Hexal
/// in the user's default terminal instead of trying to run the TUI directly.
/// </summary>
internal static class LinuxDesktopEntry
{
    private const string DesktopFileName = "hexal.desktop";

    private static bool _checked;

    public static void EnsureLauncher()
    {
        if (_checked || !OperatingSystem.IsLinux())
        {
            return;
        }

        _checked = true;

        string desktopPath = string.Empty;

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
            string? directory = Path.GetDirectoryName(executable);
            if (string.IsNullOrEmpty(directory))
            {
                LogFailure($"could not resolve the directory for '{executable}'");
                return;
            }

            if (executable.AsSpan().ContainsAny('\r', '\n'))
            {
                LogFailure("the executable path contains a line break");
                return;
            }

            desktopPath = Path.Combine(directory, DesktopFileName);
            if (File.Exists(desktopPath))
            {
                return;
            }

            using (
                var stream = new FileStream(
                    desktopPath,
                    FileMode.CreateNew,
                    FileAccess.Write,
                    FileShare.None
                )
            )
            using (var writer = new StreamWriter(stream, new UTF8Encoding(false)))
            {
                writer.Write(BuildContents(executable));
            }

            File.SetUnixFileMode(
                desktopPath,
                UnixFileMode.UserRead
                    | UnixFileMode.UserWrite
                    | UnixFileMode.UserExecute
                    | UnixFileMode.GroupRead
                    | UnixFileMode.GroupExecute
                    | UnixFileMode.OtherRead
                    | UnixFileMode.OtherExecute
            );

            HexalPaths.Log($"Created desktop launcher: {desktopPath}");
        }
        catch (IOException ex) when (desktopPath.Length > 0 && File.Exists(desktopPath))
        {
            HexalPaths.Log($"Desktop launcher already exists: {ex.Message}");
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

using Hexal.Core;
using Hexal.Languages;
using Hexal.Platform;
using Hexal.Scripting;
using Hexal.Ui;
using Hexal.Util;

namespace Hexal;

internal static class Program
{
    public const string Version = "0.1.9";

    public static int Main(string[] args) => RunHexal(args);

    public static int RunHexal(string[] arguments)
    {
        if (
            arguments.Length > 0
            && string.Equals(arguments[0], "install-plugin", StringComparison.OrdinalIgnoreCase)
        )
        {
            return RunInstallPlugin(arguments);
        }

        string file = string.Empty;
        string? directory = null;
        string? healthLanguage = null;
        bool health = false;
        bool optionsEnded = false;

        foreach (string argument in arguments)
        {
            if (!optionsEnded && argument == "--")
            {
                optionsEnded = true;
                continue;
            }

            if (!optionsEnded && (argument == "-h" || argument == "--help"))
            {
                PrintHelp();
                return 0;
            }

            if (!optionsEnded && (argument == "-v" || argument == "--version"))
            {
                Console.Out.WriteLine($"hexal {Version} ({BuildInfo.CommitHash})");
                return 0;
            }

            if (!optionsEnded && argument == "--health")
            {
                health = true;
                continue;
            }

            if (!optionsEnded && argument.Length > 1 && argument[0] == '-')
            {
                Console.Error.WriteLine($"hexal: unknown option '{argument}'");
                Console.Error.WriteLine();
                PrintHelp();
                return 2;
            }

            if (health)
            {
                if (healthLanguage is not null)
                {
                    Console.Error.WriteLine("hexal: --health accepts at most one language");
                    return 2;
                }

                healthLanguage = argument;
                continue;
            }

            if (file.Length > 0 || directory is not null)
            {
                Console.Error.WriteLine(
                    "hexal: the MVP opens one file or one project directory at a time"
                );
                return 2;
            }

            if (IsDirectory(argument))
            {
                directory = FullPath(argument);
                continue;
            }

            file = argument;
        }

        if (health)
        {
            return RunHealth(healthLanguage);
        }

        return RunEditor(file, directory);
    }

    private static int RunInstallPlugin(string[] arguments)
    {
        if (arguments.Length != 2)
        {
            Console.Error.WriteLine("usage: hexal install-plugin <name>");
            return 2;
        }

        HexalConfig config = HexalConfig.LoadDefault(out string configError);
        if (configError.Length > 0)
        {
            Console.Error.WriteLine($"hexal: config: {configError}");
        }

        string repository = string.IsNullOrWhiteSpace(config.CentralRepositoryUrl)
            ? HexalConfig.DefaultCentralRepositoryUrl
            : config.CentralRepositoryUrl;
        if (
            !PluginInstaller.TryInstall(
                arguments[1],
                repository,
                HexalPaths.PluginDirectory,
                out string installedPath,
                out string error
            )
        )
        {
            Console.Error.WriteLine($"hexal: install-plugin: {error}");
            return 1;
        }

        Console.Out.WriteLine($"Installed plugin '{arguments[1]}' to {installedPath}");
        return 0;
    }

    private static int RunHealth(string? language)
    {
        try
        {
            LanguageHealthReport report = LanguageHealth.BuildDefault();
            IReadOnlyList<LanguageHealthEntry> entries = report.Entries;

            if (!string.IsNullOrWhiteSpace(language))
            {
                LanguageHealthEntry? match = entries.FirstOrDefault(entry =>
                    string.Equals(entry.Language, language, StringComparison.OrdinalIgnoreCase)
                );
                if (match is null)
                {
                    Console.Error.WriteLine($"hexal: unknown language '{language}'");
                    return 2;
                }

                entries = new[] { match };
            }

            LanguageHealthTable.Write(entries, Console.Out, useColor: !Console.IsOutputRedirected);
            return 0;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"hexal: health check failed: {ex.Message}");
            return 1;
        }
    }

    private static bool IsDirectory(string path)
    {
        try
        {
            return Directory.Exists(path);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            return false;
        }
    }

    private static string FullPath(string path)
    {
        try
        {
            return Path.GetFullPath(path);
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return path;
        }
    }

    private static int RunEditor(string file, string? directory)
    {
        try
        {
            LinuxDesktopEntry.EnsureLauncher();

            var document = new Document();
            if (file.Length > 0)
            {
                string openPath = FileSystemUtil.ExpandHomeDirectory(file);
                Document? loaded = Document.Open(openPath, out string openError);
                if (loaded is null)
                {
                    Console.Error.WriteLine($"hexal: {openError}");
                    return 4;
                }
                document = loaded;
            }

            using ITerminal terminal = TerminalFactory.Create();
            if (!terminal.Init(out string terminalError))
            {
                Console.Error.WriteLine($"hexal: {terminalError}");
                return 1;
            }

            using var editor = new Editor(
                terminal,
                document,
                directory,
                openFilePicker: directory is not null
            );
            int result = editor.Run();
            terminal.Shutdown();
            return result;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"hexal: severe error: {ex.Message}");
            return 1;
        }
    }

    private static void PrintHelp()
    {
        Console.Out.Write(
            $"""
            Hexal v{Version} ({BuildInfo.CommitHash})
            https://hexal.pages.dev
            By Navid Momtahen (C) 2026

            Usage:
              hexal [options] [file]
              hexal [options] [directory]
              hexal install-plugin <name>

            Options:
              -h, --help        Show this help message and exit
              -v, --version     Show version information and exit
              --health [lang]   Show language support health and exit
              --                Treat the remaining arguments as file names

            Relative file names are resolved from the current working directory.
            """
        );
    }
}

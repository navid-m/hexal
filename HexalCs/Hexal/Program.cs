using Hexal.Core;
using Hexal.Languages;
using Hexal.Platform;
using Hexal.Scripting;
using Hexal.Ui;
using Hexal.Util;

namespace Hexal;

internal static class Program
{
    public const string Version = "0.1.2";

    public static int Main(string[] args) => RunHexal(args);

    public static int RunHexal(string[] arguments)
    {
        if (arguments.Length > 0 &&
            string.Equals(arguments[0], "install-plugin",
                StringComparison.OrdinalIgnoreCase))
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
                Console.Out.WriteLine($"hexal {Version}");
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
                    Console.Error.WriteLine(
                        "hexal: --health accepts at most one language");
                    return 2;
                }

                healthLanguage = argument;
                continue;
            }

            if (file.Length > 0 || directory is not null)
            {
                Console.Error.WriteLine(
                    "hexal: the MVP opens one file or one project directory at a time");
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
        if (!PluginInstaller.TryInstall(arguments[1], repository,
                HexalPaths.PluginDirectory, out string installedPath,
                out string error))
        {
            Console.Error.WriteLine($"hexal: install-plugin: {error}");
            return 1;
        }

        Console.Out.WriteLine(
            $"Installed plugin '{arguments[1]}' to {installedPath}");
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
                    string.Equals(entry.Language, language,
                        StringComparison.OrdinalIgnoreCase));
                if (match is null)
                {
                    Console.Error.WriteLine($"hexal: unknown language '{language}'");
                    return 2;
                }

                entries = new[] { match };
            }

            LanguageHealthTable.Write(entries, Console.Out);
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
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
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
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or
                                   PathTooLongException)
        {
            return path;
        }
    }

    private static int RunEditor(string file, string? directory)
    {
        try
        {
            var document = new Document();
            if (file.Length > 0)
            {
                Document? loaded = Document.Open(file, out string openError);
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

            using var editor = new Editor(terminal, document, directory,
                openFilePicker: directory is not null);
            int result = editor.Run();
            terminal.Shutdown();
            return result;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"hexal: fatal error: {ex.Message}");
            return 1;
        }
    }

    private static void PrintHelp()
    {
        Console.Out.Write(
            """
            Hexal - modal text editor

            Usage:
              hexal [options] [file]
              hexal [options] [directory]
              hexal install-plugin <name>

            Passing a directory opens the file picker rooted at that directory
            and scopes file search to it for the rest of the session.

            install-plugin downloads the named folder from the central plugin
            repository configured by centralRepositoryUrl in .hexcfg.

            Options:
              -h, --help        Show this help message and exit
              -v, --version     Show version information and exit
              --health [lang]   Show language support health and exit
              --                Treat the remaining arguments as file names

            Editor commands:
              :w [file]        Save, optionally as a new file
              :wq              Save and quit
              :q[!]            Quit (asks before discarding changes)
              :e[!] <file>     Open a file
              :lang <id>       Set the active language grammar
              :fmt             Format the current buffer
              :cw              Remove trailing whitespace from every line
              :cfg             Open the configuration file
              :rcfg            Reload configuration and grammars
              :help            Show a short key reminder
              :debug-lsp, :dl  Inspect LSP traffic and diagnostics

            Modes and keys:
              i / I            Enter INSERT mode
              Esc              Return to NORMAL mode
              h j k l          Move left / down / up / right
              arrows           Move (Home/End/PageUp/PageDown supported)
              gg / ge          Go to first / last line
              gp               Go to the previous location
              x / X            Select line; repeat to extend by one line
              v / V            Enter SELECT mode
              %                Select the whole buffer
              d / D, Delete    Delete the character or selection
              u / Ctrl+R       Undo / redo
              :                Enter command mode
              Ctrl+P           Open the command palette
              /                Search inside the current file
              .                Select all matches for multi-cursor editing
              Space            Open the space menu
              Space /          Search across files
              Space f          Open the file picker
              Space y          Yank the selection to the system clipboard
              Space p          Paste the system clipboard on the current line
              Space P          Paste the system clipboard on a new line
              Space K          Show LSP hover information
              Space A          Show and apply LSP code actions
              Space S          Pick a symbol in the current file
              Space D          Show workspace diagnostics
              Space d          Show diagnostics for the current file

            Relative file names are resolved from the current working directory.

            """);
    }
}

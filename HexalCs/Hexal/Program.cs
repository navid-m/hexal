using Hexal.Core;
using Hexal.Platform;
using Hexal.Ui;

namespace Hexal;

internal static class Program
{
    private const string Version = "0.1.0";

    public static int Main(string[] args) => RunHexal(args);

    public static int RunHexal(string[] arguments)
    {
        string file = string.Empty;
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

            if (!optionsEnded && argument.Length > 1 && argument[0] == '-')
            {
                Console.Error.WriteLine($"hexal: unknown option '{argument}'");
                Console.Error.WriteLine();
                PrintHelp();
                return 2;
            }

            if (file.Length > 0)
            {
                Console.Error.WriteLine(
                    "hexal: the MVP opens one file at a time; use :e to switch files");
                return 2;
            }

            file = argument;
        }

        return RunEditor(file);
    }

    private static int RunEditor(string file)
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

            using var editor = new Editor(terminal, document);
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
            Hexal - modal text editor (MVP)

            Usage:
              hexal [options] [file]

            Options:
              -h, --help       Show this help message and exit
              -v, --version    Show version information and exit
              --               Treat the remaining arguments as file names

            Editor commands:
              :w [file]        Save, optionally as a new file
              :wq              Save and quit
              :q[!]            Quit (asks before discarding changes)
              :e[!] <file>     Open a file
              :lang <id>       Set the active language grammar
              :help            Show a short key reminder

            Modes and keys:
              i / I            Enter INSERT mode
              Esc              Return to NORMAL mode
              h j k l          Move left / down / up / right
              arrows           Move (Home/End/PageUp/PageDown supported)
              gg / ge          Go to first / last line
              gp               Go to the previous location
              x / X            Select line; repeat to extend by one line
              v / V            Enter SELECT mode
              d / D, Delete    Delete the character or selection
              u / Ctrl+R       Undo / redo
              :                Enter command mode
              Space            Open the space menu
              Space /          Search across files
              Space f          Open the file picker

            Relative file names are resolved from the current working directory.

            """);
    }
}

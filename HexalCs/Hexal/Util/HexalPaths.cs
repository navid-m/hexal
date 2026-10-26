namespace Hexal.Util;

/// <summary>
/// Resolves portable data directories next to the executable, falling back to
/// the per-user ~/.hexal/data tree. Resolution happens once per process and is
/// recorded in ~/.hexal/hexal.log.
/// </summary>
public static class HexalPaths
{
    private const long MaxLogBytes = 1024 * 1024;

    private static readonly object Lock = new();
    private static bool _resolved;
    private static string _grammarDirectory = string.Empty;
    private static string _pluginDirectory = string.Empty;
    private static string _themeDirectory = string.Empty;

    public static string HomeDirectory
    {
        get
        {
            string home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            if (string.IsNullOrEmpty(home))
            {
                home = Environment.GetEnvironmentVariable("HOME") ?? ".";
            }
            return home;
        }
    }

    public static string HexalHomeDirectory => Path.Combine(HomeDirectory, ".hexal");

    public static string LogPath => Path.Combine(HexalHomeDirectory, "hexal.log");

    public static string BinaryDirectory
    {
        get
        {
            string directory = AppContext.BaseDirectory;
            if (string.IsNullOrEmpty(directory))
            {
                directory = Directory.GetCurrentDirectory();
            }

            try
            {
                return Path.GetFullPath(directory);
            }
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                return directory;
            }
        }
    }

    public static string BinaryDataDirectory => Path.Combine(BinaryDirectory, "data");

    public static string FallbackDataDirectory => Path.Combine(HexalHomeDirectory, "data");

    public static string GrammarDirectory
    {
        get
        {
            EnsureResolved();
            return _grammarDirectory;
        }
    }

    public static string PluginDirectory
    {
        get
        {
            EnsureResolved();
            return _pluginDirectory;
        }
    }

    public static string ThemeDirectory
    {
        get
        {
            EnsureResolved();
            return _themeDirectory;
        }
    }

    public static string ActiveThemePath => Path.Combine(HexalHomeDirectory, "theme");

    public static string ResolveDataDirectory(
        string binaryDirectory,
        string homeDirectory,
        string name
    )
    {
        string portable = Path.Combine(binaryDirectory, "data", name);
        if (Directory.Exists(portable))
        {
            return portable;
        }

        return Path.Combine(homeDirectory, ".hexal", "data", name);
    }

    public static void Log(string message)
    {
        try
        {
            Directory.CreateDirectory(HexalHomeDirectory);
            string path = LogPath;

            var info = new FileInfo(path);
            if (info.Exists && info.Length > MaxLogBytes)
            {
                File.WriteAllText(path, string.Empty);
            }

            File.AppendAllText(
                path,
                $"{DateTime.UtcNow:yyyy-MM-ddTHH:mm:ss.fffZ} {message}{Environment.NewLine}"
            );
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            // Logging must never prevent the editor from starting.
        }
    }

    private static void EnsureResolved()
    {
        lock (Lock)
        {
            if (_resolved)
            {
                return;
            }
            _resolved = true;

            long resolvedStarted = StartupTrace.Timestamp();
            long phaseStarted = StartupTrace.Timestamp();
            string binaryData = BinaryDataDirectory;
            StartupTrace.Mark("paths: binary data directory", phaseStarted, binaryData);

            string portableGrammars = Path.Combine(binaryData, "grammars");
            string portablePlugins = Path.Combine(binaryData, "plugins");
            string portableThemes = Path.Combine(binaryData, "themes");

            phaseStarted = StartupTrace.Timestamp();
            bool hasPortableGrammars = Directory.Exists(portableGrammars);
            StartupTrace.Mark(
                "paths: check portable grammars",
                phaseStarted,
                $"{hasPortableGrammars} {portableGrammars}"
            );

            phaseStarted = StartupTrace.Timestamp();
            bool hasPortablePlugins = Directory.Exists(portablePlugins);
            StartupTrace.Mark(
                "paths: check portable plugins",
                phaseStarted,
                $"{hasPortablePlugins} {portablePlugins}"
            );

            phaseStarted = StartupTrace.Timestamp();
            bool hasPortableThemes = Directory.Exists(portableThemes);
            StartupTrace.Mark(
                "paths: check portable themes",
                phaseStarted,
                $"{hasPortableThemes} {portableThemes}"
            );

            string homeData = FallbackDataDirectory;
            _grammarDirectory = hasPortableGrammars
                ? portableGrammars
                : Path.Combine(homeData, "grammars");
            _pluginDirectory = hasPortablePlugins
                ? portablePlugins
                : Path.Combine(homeData, "plugins");
            _themeDirectory = hasPortableThemes ? portableThemes : Path.Combine(homeData, "themes");

            phaseStarted = StartupTrace.Timestamp();
            Log($"Hexal binary directory: {BinaryDirectory}");
            StartupTrace.Mark("paths: log binary directory", phaseStarted);

            phaseStarted = StartupTrace.Timestamp();
            if (!hasPortableGrammars && !hasPortablePlugins && !hasPortableThemes)
            {
                Log(
                    "Could not find grammars, plugins and themes relative to binary path, "
                        + "falling back to ~/.hexal directory"
                );
            }
            else
            {
                Log(
                    hasPortableGrammars
                        ? $"Using grammars from binary-relative path: {_grammarDirectory}"
                        : $"Could not find grammars relative to binary path, falling back to {_grammarDirectory}"
                );
                Log(
                    hasPortablePlugins
                        ? $"Using plugins from binary-relative path: {_pluginDirectory}"
                        : $"Could not find plugins relative to binary path, falling back to {_pluginDirectory}"
                );
                Log(
                    hasPortableThemes
                        ? $"Using themes from binary-relative path: {_themeDirectory}"
                        : $"Could not find themes relative to binary path, falling back to {_themeDirectory}"
                );
            }
            StartupTrace.Mark("paths: log resolution details", phaseStarted);
            StartupTrace.Mark("paths: resolution total", resolvedStarted);
        }
    }
}

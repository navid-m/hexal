using System.Text;
using System.Text.RegularExpressions;

namespace Hexal.Util;

/// <summary>
/// Matches paths against <c>.gitignore</c> files below a project root. Rules
/// are loaded lazily per directory and follow the usual Git semantics:
/// nested files override their parents, a leading slash anchors a pattern to
/// the directory that declares it, and a trailing slash restricts a pattern to
/// directories. Patterns without a slash match a path component at any depth.
/// </summary>
public sealed class GitIgnore
{
    private const long MaxGitIgnoreBytes = 1L * 1024 * 1024;

    private static readonly bool CaseInsensitivePaths = OperatingSystem.IsWindows();

    private static readonly StringComparer PathComparison = CaseInsensitivePaths
        ? StringComparer.OrdinalIgnoreCase
        : StringComparer.Ordinal;

    private static readonly RegexOptions PatternOptions =
        RegexOptions.CultureInvariant
        | (
            OperatingSystem.IsWindows() || OperatingSystem.IsMacOS()
                ? RegexOptions.IgnoreCase
                : RegexOptions.None
        );

    private static readonly HashSet<string> BuiltInIgnoredDirectories = new(
        StringComparer.OrdinalIgnoreCase
    )
    {
        ".git",
        ".hg",
        ".svn",
        ".idea",
        ".vs",
        ".vscode",
        "bin",
        "obj",
        "build",
        "dist",
        "target",
        "node_modules",
        "vendor",
    };

    private sealed class Rule
    {
        public string BaseDirectory = string.Empty;
        public Regex Pattern = null!;
        public bool Anchored;
        public bool DirectoryOnly;
        public bool Negated;
    }

    private readonly string _root;
    private readonly Dictionary<string, Rule[]> _rulesByDirectory = new(PathComparison);

    public GitIgnore(string root)
    {
        _root = Normalize(root);
    }

    public static bool IsBuiltInIgnoredDirectory(string name) =>
        BuiltInIgnoredDirectories.Contains(name);

    /// <summary>
    /// Returns true when <paramref name="fullPath"/> is excluded by a
    /// <c>.gitignore</c> file (or by the repository's info/exclude file).
    /// </summary>
    public bool IsIgnored(string fullPath, bool isDirectory)
    {
        string full;
        try
        {
            full = Path.GetFullPath(fullPath);
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return false;
        }

        if (SamePath(full, _root) || !IsInsideRoot(full))
        {
            return false;
        }

        string relative = ToGitPath(Path.GetRelativePath(_root, full));
        if (relative.Length == 0 || relative == ".")
        {
            return false;
        }

        List<Rule> rules = RulesForPath(full);
        if (rules.Count == 0)
        {
            return false;
        }

        bool ignored = false;
        foreach (Rule rule in rules)
        {
            string baseRelative = ToGitPath(Path.GetRelativePath(rule.BaseDirectory, full));
            if (
                baseRelative.Length == 0
                || baseRelative == "."
                || baseRelative.StartsWith("../", StringComparison.Ordinal)
            )
            {
                continue;
            }

            bool matched;
            if (rule.Anchored)
            {
                matched = MatchesAnchored(rule, baseRelative, isDirectory);
            }
            else
            {
                matched = MatchesComponent(rule, Split(baseRelative), isDirectory);
            }

            if (matched)
            {
                ignored = !rule.Negated;
            }
        }

        return ignored;
    }

    private static bool MatchesAnchored(Rule rule, string relative, bool isDirectory)
    {
        string[] components = Split(relative);
        for (int index = 1; index < components.Length; ++index)
        {
            string ancestor = string.Join('/', components, 0, index);
            if (rule.Pattern.IsMatch(ancestor))
            {
                return true;
            }
        }

        if (rule.DirectoryOnly && !isDirectory)
        {
            return false;
        }

        return rule.Pattern.IsMatch(string.Join('/', components));
    }

    private static bool MatchesComponent(Rule rule, string[] components, bool isDirectory)
    {
        for (int index = 0; index < components.Length; ++index)
        {
            bool componentIsDirectory = isDirectory || index < components.Length - 1;
            if (rule.DirectoryOnly && !componentIsDirectory)
            {
                continue;
            }

            if (rule.Pattern.IsMatch(components[index]))
            {
                return true;
            }
        }

        return false;
    }

    private List<Rule> RulesForPath(string fullPath)
    {
        var directories = new Stack<string>();
        string? directory = Path.GetDirectoryName(fullPath);
        while (!string.IsNullOrEmpty(directory) && IsInsideOrRoot(directory))
        {
            directories.Push(directory);
            if (SamePath(directory, _root))
            {
                break;
            }

            directory = Path.GetDirectoryName(directory);
        }

        var rules = new List<Rule>();
        while (directories.Count > 0)
        {
            rules.AddRange(RulesForDirectory(directories.Pop()));
        }

        return rules;
    }

    private Rule[] RulesForDirectory(string directory)
    {
        directory = Normalize(directory);
        if (_rulesByDirectory.TryGetValue(directory, out Rule[]? cached))
        {
            return cached;
        }

        var rules = new List<Rule>();
        if (SamePath(directory, _root))
        {
            LoadRules(Path.Combine(directory, ".git", "info", "exclude"), directory, rules);
        }

        LoadRules(Path.Combine(directory, ".gitignore"), directory, rules);

        Rule[] result = rules.ToArray();
        _rulesByDirectory[directory] = result;
        return result;
    }

    private static void LoadRules(string path, string baseDirectory, List<Rule> rules)
    {
        string[] lines;
        try
        {
            var info = new FileInfo(path);
            if (!info.Exists || info.Length > MaxGitIgnoreBytes)
            {
                return;
            }

            lines = File.ReadAllLines(path);
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
                        or PathTooLongException
            )
        {
            return;
        }

        foreach (string line in lines)
        {
            Rule? rule = ParseRule(line, baseDirectory);
            if (rule is not null)
            {
                rules.Add(rule);
            }
        }
    }

    private static Rule? ParseRule(string line, string baseDirectory)
    {
        if (line.Length == 0)
        {
            return null;
        }

        int end = line.Length;
        while (end > 0 && line[end - 1] == ' ')
        {
            int backslashes = 0;
            for (int index = end - 2; index >= 0 && line[index] == '\\'; --index)
            {
                ++backslashes;
            }

            if ((backslashes & 1) == 1)
            {
                break;
            }

            --end;
        }

        if (end == 0)
        {
            return null;
        }

        line = line[..end];
        if (line[0] == '#')
        {
            return null;
        }

        bool negated = false;
        if (line[0] == '!')
        {
            negated = true;
            line = line[1..];
        }
        else if (line.Length > 1 && line[0] == '\\' && line[1] is '#' or '!')
        {
            line = line[1..];
        }

        if (line.Length == 0)
        {
            return null;
        }

        bool directoryOnly = line.EndsWith('/');
        if (directoryOnly)
        {
            line = line[..^1];
        }

        if (line.Length == 0)
        {
            return null;
        }

        bool anchored = line.StartsWith('/');
        if (anchored)
        {
            line = line[1..];
        }
        else if (line.Contains('/'))
        {
            anchored = true;
        }

        if (line.Length == 0)
        {
            return null;
        }

        try
        {
            return new Rule
            {
                BaseDirectory = baseDirectory,
                Pattern = new Regex(BuildPattern(line), PatternOptions),
                Anchored = anchored,
                DirectoryOnly = directoryOnly,
                Negated = negated,
            };
        }
        catch (ArgumentException)
        {
            return null;
        }
    }

    private static string BuildPattern(string glob)
    {
        var pattern = new StringBuilder("^");
        int index = 0;
        while (index < glob.Length)
        {
            char character = glob[index];
            switch (character)
            {
                case '\\':
                    if (index + 1 < glob.Length)
                    {
                        pattern.Append(Regex.Escape(glob[index + 1].ToString()));
                        index += 2;
                    }
                    else
                    {
                        pattern.Append("\\\\");
                        ++index;
                    }
                    break;

                case '*':
                    int start = index;
                    while (index < glob.Length && glob[index] == '*')
                    {
                        ++index;
                    }

                    int count = index - start;
                    bool followedBySlash = index < glob.Length && glob[index] == '/';
                    if (count >= 2)
                    {
                        if (followedBySlash)
                        {
                            pattern.Append("(?:[^/]+/)*");
                            ++index;
                        }
                        else
                        {
                            pattern.Append(".*");
                        }
                    }
                    else
                    {
                        pattern.Append("[^/]*");
                    }
                    break;

                case '?':
                    pattern.Append("[^/]");
                    ++index;
                    break;

                case '[':
                    int close = FindClassEnd(glob, index + 1);
                    if (close < 0)
                    {
                        pattern.Append("\\[");
                        ++index;
                        break;
                    }

                    pattern.Append('[');
                    int part = index + 1;
                    if (part < close && glob[part] is '!' or '^')
                    {
                        pattern.Append('^');
                        ++part;
                    }

                    for (; part < close; ++part)
                    {
                        char member = glob[part];
                        if (member == '\\' && part + 1 < close)
                        {
                            pattern.Append(Regex.Escape(glob[part + 1].ToString()));
                            ++part;
                        }
                        else if (member is ']' or '[')
                        {
                            pattern.Append('\\').Append(member);
                        }
                        else
                        {
                            pattern.Append(member);
                        }
                    }

                    pattern.Append(']');
                    index = close + 1;
                    break;

                default:
                    pattern.Append(Regex.Escape(character.ToString()));
                    ++index;
                    break;
            }
        }

        pattern.Append('$');
        return pattern.ToString();
    }

    private static int FindClassEnd(string glob, int start)
    {
        for (int index = start; index < glob.Length; ++index)
        {
            if (glob[index] == '\\')
            {
                ++index;
                continue;
            }

            if (glob[index] == ']')
            {
                return index;
            }
        }

        return -1;
    }

    private static string[] Split(string path) =>
        path.Split('/', StringSplitOptions.RemoveEmptyEntries);

    private static string ToGitPath(string path) => path.Replace('\\', '/');

    private static string Normalize(string path)
    {
        try
        {
            return Path.TrimEndingDirectorySeparator(Path.GetFullPath(path));
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return Path.TrimEndingDirectorySeparator(path);
        }
    }

    private bool IsInsideRoot(string path) => IsInsideOrRoot(path) && !SamePath(path, _root);

    private bool IsInsideOrRoot(string path)
    {
        if (SamePath(path, _root))
        {
            return true;
        }

        string prefix = _root.EndsWith(Path.DirectorySeparatorChar)
            ? _root
            : _root + Path.DirectorySeparatorChar;
        return path.StartsWith(
            prefix,
            CaseInsensitivePaths ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal
        );
    }

    private static bool SamePath(string left, string right) =>
        string.Equals(
            left,
            right,
            CaseInsensitivePaths ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal
        );
}

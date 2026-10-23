using System.Text;

namespace Hexal.Util;

/// <summary>
/// A text match found by <see cref="FileSearch.Search"/>.
/// <see cref="Line"/> and <see cref="Column"/> are zero-based; the column is a
/// UTF-8 byte offset within the line, matching the editor's cursor model.
/// </summary>
public readonly struct FileSearchMatch
{
    public string Path { get; init; }
    public long Line { get; init; }
    public long Column { get; init; }
    public string Preview { get; init; }
}

/// <summary>
/// The first few lines of a file, used by the file-picker preview pane.
/// </summary>
public sealed class FilePreview
{
    public string Path { get; init; } = string.Empty;
    public IReadOnlyList<string> Lines { get; init; } = Array.Empty<string>();
    public bool Binary { get; init; }
    public bool Truncated { get; init; }
    public string Error { get; init; } = string.Empty;
}

/// <summary>
/// Shared project-file enumeration and plain-text search used by the file
/// picker and the workspace search modal. The implementation deliberately has
/// no UI dependency so it can run on a worker thread.
/// </summary>
public static class FileSearch
{
    public const int DefaultMaxFiles = 20_000;
    public const int DefaultMaxResults = 1_000;
    public const int DefaultPreviewLines = 10;
    private const int MaxPreviewLineLength = 400;
    private const long MaxSearchFileBytes = 4L * 1024 * 1024;
    private const int MaxPreviewLength = 200;

    /// <summary>
    /// Enumerates regular files below <paramref name="root"/>, returning paths
    /// relative to the root. Ignored directories and directory symlinks are
    /// skipped, and the result is sorted for deterministic display.
    /// </summary>
    public static List<string> EnumerateFiles(string root, int maxFiles,
        CancellationToken cancellationToken)
        => EnumerateFiles(root, maxFiles, showIgnored: false, cancellationToken);

    /// <summary>
    /// Enumerates regular files, optionally including paths excluded by
    /// <c>.gitignore</c> and the built-in build/VCS directory list.
    /// </summary>
    public static List<string> EnumerateFiles(string root, int maxFiles,
        bool showIgnored, CancellationToken cancellationToken)
    {
        var result = new List<string>();
        if (maxFiles <= 0 || string.IsNullOrEmpty(root) || !Directory.Exists(root))
        {
            return result;
        }

        string fullRoot;
        try
        {
            fullRoot = Path.GetFullPath(root);
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return result;
        }

        var gitIgnore = new GitIgnore(fullRoot);
        var pending = new Stack<string>();
        pending.Push(fullRoot);

        while (pending.Count > 0 && result.Count < maxFiles)
        {
            cancellationToken.ThrowIfCancellationRequested();
            string directory = pending.Pop();

            string[] files;
            string[] directories;
            try
            {
                files = Directory.GetFiles(directory);
                directories = Directory.GetDirectories(directory);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException)
            {
                continue;
            }

            foreach (string file in files)
            {
                cancellationToken.ThrowIfCancellationRequested();
                if (!showIgnored && gitIgnore.IsIgnored(file, isDirectory: false))
                {
                    continue;
                }

                result.Add(Path.GetRelativePath(fullRoot, file));
                if (result.Count >= maxFiles)
                {
                    break;
                }
            }

            foreach (string child in directories)
            {
                cancellationToken.ThrowIfCancellationRequested();
                string name = Path.GetFileName(child);
                if (!showIgnored &&
                    (GitIgnore.IsBuiltInIgnoredDirectory(name) ||
                     gitIgnore.IsIgnored(child, isDirectory: true)))
                {
                    continue;
                }

                try
                {
                    FileAttributes attributes = File.GetAttributes(child);
                    if ((attributes & FileAttributes.ReparsePoint) != 0)
                    {
                        continue;
                    }
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                           NotSupportedException or ArgumentException)
                {
                    continue;
                }

                pending.Push(child);
            }
        }

        result.Sort(StringComparer.OrdinalIgnoreCase);
        return result;
    }

    /// <summary>
    /// Searches the supplied relative paths for a case-insensitive plain-text
    /// match. Binary-looking and very large files are skipped. Results stop at
    /// <paramref name="maxResults"/> so a common query cannot exhaust memory.
    /// </summary>
    public static List<FileSearchMatch> Search(string root,
        IReadOnlyList<string> relativePaths, string query, int maxResults,
        CancellationToken cancellationToken)
    {
        var matches = new List<FileSearchMatch>();
        if (query.Length == 0 || maxResults <= 0)
        {
            return matches;
        }

        foreach (string relativePath in relativePaths)
        {
            cancellationToken.ThrowIfCancellationRequested();

            string fullPath;
            try
            {
                fullPath = Path.Combine(root, relativePath);
                var info = new FileInfo(fullPath);
                if (!info.Exists || info.Length > MaxSearchFileBytes)
                {
                    continue;
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException or
                                       PathTooLongException)
            {
                continue;
            }

            try
            {
                long lineNumber = 0;

                foreach (string line in File.ReadLines(fullPath))
                {
                    if ((lineNumber & 0x3F) == 0)
                    {
                        cancellationToken.ThrowIfCancellationRequested();
                    }

                    // A NUL byte is a good enough heuristic for a binary file.
                    if (line.Contains('\0'))
                    {
                        break;
                    }

                    int index = line.IndexOf(query, StringComparison.OrdinalIgnoreCase);
                    if (index >= 0)
                    {
                        matches.Add(new FileSearchMatch
                        {
                            Path = relativePath,
                            Line = lineNumber,
                            Column = Encoding.UTF8.GetByteCount(line.AsSpan(0, index)),
                            Preview = MakePreview(line, index)
                        });

                        if (matches.Count >= maxResults)
                        {
                            return matches;
                        }
                    }

                    ++lineNumber;
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException)
            {
                // An unreadable file must not abort the whole search.
            }
        }

        return matches;
    }

    /// <summary>
    /// Reads the leading lines of a file for the picker preview. Oversized
    /// lines are clipped so a single minified line cannot stall the editor.
    /// </summary>
    public static FilePreview ReadPreview(string path,
        int maxLines = DefaultPreviewLines)
    {
        if (string.IsNullOrEmpty(path))
        {
            return new FilePreview { Error = "no file selected" };
        }

        try
        {
            if (!File.Exists(path))
            {
                return new FilePreview { Path = path, Error = "file does not exist" };
            }

            var info = new FileInfo(path);
            if (info.Length > MaxSearchFileBytes)
            {
                return new FilePreview
                {
                    Path = path,
                    Error = "file is too large to preview"
                };
            }

            var lines = new List<string>(Math.Max(1, maxLines));
            bool binary = false;
            bool truncated = false;

            using var stream = new FileStream(path, FileMode.Open, FileAccess.Read,
                FileShare.ReadWrite);
            using var reader = new StreamReader(stream, Encoding.UTF8,
                detectEncodingFromByteOrderMarks: true);

            while (lines.Count < maxLines)
            {
                string? line = reader.ReadLine();
                if (line is null)
                {
                    break;
                }

                if (line.Contains('\0'))
                {
                    binary = true;
                    break;
                }

                if (line.Length > MaxPreviewLineLength)
                {
                    line = line[..MaxPreviewLineLength] + "…";
                    truncated = true;
                }

                lines.Add(line);
            }

            if (!binary && lines.Count >= maxLines && reader.ReadLine() is not null)
            {
                truncated = true;
            }

            return new FilePreview
            {
                Path = path,
                Lines = lines,
                Binary = binary,
                Truncated = truncated
            };
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException or
                                   PathTooLongException)
        {
            return new FilePreview { Path = path, Error = ex.Message };
        }
    }

    private static string MakePreview(string line, int index)
    {
        string trimmed = line.Trim();
        if (trimmed.Length <= MaxPreviewLength)
        {
            return trimmed;
        }

        int leading = line.Length - line.TrimStart().Length;
        int relative = Math.Max(0, index - leading);
        int start = Math.Max(0, relative - (MaxPreviewLength / 2));
        start = Math.Min(start, Math.Max(0, trimmed.Length - MaxPreviewLength));

        string preview = trimmed.Substring(start, MaxPreviewLength);
        if (start > 0)
        {
            preview = "…" + preview[1..];
        }
        if (start + MaxPreviewLength < trimmed.Length)
        {
            preview = preview[..^1] + "…";
        }

        return preview;
    }
}

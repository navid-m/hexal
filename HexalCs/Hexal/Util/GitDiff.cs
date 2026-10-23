using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace Hexal.Util;

public enum GitDiffLineKind
{
    Deleted,
    Added,
    Modified
}

/// <summary>
/// Reads the line-level changes for a file from Git and turns zero-context
/// diff hunks into gutter markers. The working tree is compared with HEAD so
/// both staged and unstaged changes are included.
/// </summary>
public static class GitDiff
{
    private const int HelperTimeoutMilliseconds = 3000;
    private const int LiveDiffMaxLines = 5_000;
    private const int LiveDiffMaxDepth = 2_000;
    private const long LiveDiffMemoryBudget = 32L * 1024 * 1024;

    private static readonly Regex HunkHeader = new(
        @"^@@ -(?<oldStart>\d+)(?:,(?<oldCount>\d+))? " +
        @"\+(?<newStart>\d+)(?:,(?<newCount>\d+))? @@",
        RegexOptions.Compiled | RegexOptions.CultureInvariant);

    public static Dictionary<long, GitDiffLineKind> Load(string? path,
        long lineCount)
    {
        var result = new Dictionary<long, GitDiffLineKind>();
        if (string.IsNullOrEmpty(path) || lineCount <= 0)
        {
            return result;
        }

        string? directory = Path.GetDirectoryName(path);
        string fileName = Path.GetFileName(path);
        if (string.IsNullOrEmpty(directory) || fileName.Length == 0)
        {
            return result;
        }

        string? output = Run(directory, fileName, useHead: true);
        if (output is null)
        {
            output = Run(directory, fileName, useHead: false);
        }

        if (!string.IsNullOrEmpty(output))
        {
            Parse(output, lineCount, result);
        }

        return result;
    }

    /// <summary>
    /// True when the file is small enough for live in-memory diffing.
    /// </summary>
    public static bool CanComputeLive(long lineCount)
        => lineCount <= LiveDiffMaxLines;

    /// <summary>
    /// Reads the committed version of a file. Used as the baseline for live
    /// diffing while the buffer contains unsaved edits.
    /// </summary>
    public static ulong[]? LoadBaseline(string path)
    {
        if (string.IsNullOrEmpty(path))
        {
            return null;
        }

        string? directory = Path.GetDirectoryName(path);
        string fileName = Path.GetFileName(path);
        if (string.IsNullOrEmpty(directory) || fileName.Length == 0)
        {
            return null;
        }

        byte[]? content = RunShow(directory, "HEAD:./" + fileName);
        return content is null ? null : HashLines(content);
    }

    /// <summary>
    /// Computes gutter markers from line hashes. Lines are compared with a
    /// Myers diff so unsaved insertions, deletions and replacements are shown
    /// immediately.
    /// </summary>
    public static Dictionary<long, GitDiffLineKind> Compute(ulong[] oldLines,
        ulong[] newLines)
    {
        var result = new Dictionary<long, GitDiffLineKind>();
        if (oldLines.Length == 0 && newLines.Length == 0)
        {
            return result;
        }

        if (oldLines.Length + newLines.Length > LiveDiffMaxLines * 2)
        {
            return result;
        }

        int prefix = 0;
        while (prefix < oldLines.Length && prefix < newLines.Length &&
               oldLines[prefix] == newLines[prefix])
        {
            ++prefix;
        }

        int oldEnd = oldLines.Length;
        int newEnd = newLines.Length;
        while (oldEnd > prefix && newEnd > prefix &&
               oldLines[oldEnd - 1] == newLines[newEnd - 1])
        {
            --oldEnd;
            --newEnd;
        }

        ulong[] middleOld = oldLines[prefix..oldEnd];
        ulong[] middleNew = newLines[prefix..newEnd];
        if (middleOld.Length == 0 && middleNew.Length == 0)
        {
            return result;
        }

        List<EditKind>? edits = Myers(middleOld, middleNew);
        if (edits is null)
        {
            MarkLargeChange(result, middleOld.Length, prefix, newEnd,
                newLines.Length);
            return result;
        }

        long newIndex = prefix;
        long blockNewStart = 0;
        long blockDeletes = 0;
        long blockInserts = 0;
        bool inBlock = false;

        foreach (EditKind edit in edits)
        {
            if (edit == EditKind.Equal)
            {
                if (inBlock)
                {
                    MarkBlock(result, blockNewStart, blockDeletes,
                        blockInserts, newLines.Length);
                    inBlock = false;
                    blockDeletes = 0;
                    blockInserts = 0;
                }

                ++newIndex;
                continue;
            }

            if (!inBlock)
            {
                inBlock = true;
                blockNewStart = newIndex;
            }

            if (edit == EditKind.Delete)
            {
                ++blockDeletes;
            }
            else
            {
                ++blockInserts;
                ++newIndex;
            }
        }

        if (inBlock)
        {
            MarkBlock(result, blockNewStart, blockDeletes, blockInserts,
                newLines.Length);
        }

        return result;
    }

    public static ulong HashLine(ReadOnlySpan<byte> line)
    {
        ulong hash = 14695981039346656037UL;
        foreach (byte value in line)
        {
            hash ^= value;
            hash *= 1099511628211UL;
        }
        return hash;
    }

    private static void MarkBlock(Dictionary<long, GitDiffLineKind> result,
        long newStart, long deletes, long inserts, long newLineCount)
    {
        long common = Math.Min(deletes, inserts);
        for (long index = 0; index < common; ++index)
        {
            result[newStart + index] = GitDiffLineKind.Modified;
        }

        for (long index = common; index < inserts; ++index)
        {
            result[newStart + index] = GitDiffLineKind.Added;
        }

        if (deletes > inserts && newLineCount > 0)
        {
            long line = Math.Clamp(newStart + inserts, 0, newLineCount - 1);
            result[line] = GitDiffLineKind.Deleted;
        }
    }

    private static ulong[] HashLines(byte[] content)
    {
        var hashes = new List<ulong>();
        int start = content.Length >= 3 &&
                    content[0] == 0xEF && content[1] == 0xBB && content[2] == 0xBF
            ? 3
            : 0;
        for (int index = start; index < content.Length; ++index)
        {
            if (content[index] != (byte)'\n')
            {
                continue;
            }

            int end = index;
            if (end > start && content[end - 1] == (byte)'\r')
            {
                --end;
            }

            hashes.Add(HashLine(content.AsSpan(start, end - start)));
            start = index + 1;
        }

        hashes.Add(HashLine(content.AsSpan(start)));
        return hashes.ToArray();
    }

    private enum EditKind : byte
    {
        Equal,
        Delete,
        Insert
    }

    private static List<EditKind>? Myers(ulong[] oldLines, ulong[] newLines)
    {
        int oldCount = oldLines.Length;
        int newCount = newLines.Length;
        int max = oldCount + newCount;
        int offset = max;
        var frontier = new int[max * 2 + 1];
        long frontierBytes = (long)frontier.Length * sizeof(int);
        int depthBudget = (int)Math.Min(max, Math.Min(LiveDiffMaxDepth,
            LiveDiffMemoryBudget / Math.Max(1L, frontierBytes)));
        var trace = new List<int[]>();

        for (int depth = 0; depth <= max; ++depth)
        {
            if (depth > depthBudget)
            {
                return null;
            }

            trace.Add((int[])frontier.Clone());
            for (int diagonal = -depth; diagonal <= depth; diagonal += 2)
            {
                int x;
                if (diagonal == -depth ||
                    (diagonal != depth &&
                     frontier[offset + diagonal - 1] <
                     frontier[offset + diagonal + 1]))
                {
                    x = frontier[offset + diagonal + 1];
                }
                else
                {
                    x = frontier[offset + diagonal - 1] + 1;
                }

                int y = x - diagonal;
                while (x < oldCount && y < newCount &&
                       oldLines[x] == newLines[y])
                {
                    ++x;
                    ++y;
                }

                frontier[offset + diagonal] = x;
                if (x >= oldCount && y >= newCount)
                {
                    return Backtrack(trace, depth, offset, oldCount, newCount);
                }
            }
        }

        return new List<EditKind>();
    }

    private static void MarkLargeChange(Dictionary<long, GitDiffLineKind> result,
        int oldCount, int newStart, int newEnd, int newLineCount)
    {
        if (newStart >= newEnd)
        {
            if (newLineCount > 0)
            {
                result[Math.Clamp(newStart, 0, newLineCount - 1)] =
                    GitDiffLineKind.Deleted;
            }
            return;
        }

        GitDiffLineKind kind = oldCount == 0
            ? GitDiffLineKind.Added
            : GitDiffLineKind.Modified;
        for (int line = newStart; line < newEnd; ++line)
        {
            result[line] = kind;
        }
    }

    private static List<EditKind> Backtrack(List<int[]> trace, int depth,
        int offset, int oldCount, int newCount)
    {
        var edits = new List<EditKind>(oldCount + newCount);
        int x = oldCount;
        int y = newCount;

        for (int current = depth; current >= 0; --current)
        {
            int[] frontier = trace[current];
            int diagonal = x - y;
            int previousDiagonal;
            if (diagonal == -current ||
                (diagonal != current &&
                 frontier[offset + diagonal - 1] <
                 frontier[offset + diagonal + 1]))
            {
                previousDiagonal = diagonal + 1;
            }
            else
            {
                previousDiagonal = diagonal - 1;
            }

            int previousX = frontier[offset + previousDiagonal];
            int previousY = previousX - previousDiagonal;

            while (x > previousX && y > previousY)
            {
                edits.Add(EditKind.Equal);
                --x;
                --y;
            }

            if (current == 0)
            {
                break;
            }

            if (x == previousX)
            {
                edits.Add(EditKind.Insert);
            }
            else
            {
                edits.Add(EditKind.Delete);
            }

            x = previousX;
            y = previousY;
        }

        edits.Reverse();
        return edits;
    }

    private static byte[]? RunShow(string directory, string revisionPath)
    {
        try
        {
            var startInfo = new ProcessStartInfo
            {
                FileName = "git",
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true
            };

            startInfo.ArgumentList.Add("-C");
            startInfo.ArgumentList.Add(directory);
            startInfo.ArgumentList.Add("show");
            startInfo.ArgumentList.Add(revisionPath);

            using Process? process = Process.Start(startInfo);
            if (process is null)
            {
                return null;
            }

            process.StandardInput.Close();
            Task<byte[]> outputTask = ReadAllBytesAsync(process.StandardOutput.BaseStream);
            Task<string> errorTask = process.StandardError.ReadToEndAsync();

            if (!process.WaitForExit(HelperTimeoutMilliseconds))
            {
                try
                {
                    process.Kill(entireProcessTree: true);
                }
                catch (Exception ex) when (ex is InvalidOperationException or
                                           NotSupportedException or
                                           System.ComponentModel.Win32Exception)
                {
                    // The process may already have exited.
                }

                return null;
            }

            byte[] output = outputTask.GetAwaiter().GetResult();
            _ = errorTask.GetAwaiter().GetResult();
            return process.ExitCode == 0 ? output : null;
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or
                                   IOException or InvalidOperationException or
                                   NotSupportedException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    private static async Task<byte[]> ReadAllBytesAsync(Stream stream)
    {
        using var buffer = new MemoryStream();
        await stream.CopyToAsync(buffer).ConfigureAwait(false);
        return buffer.ToArray();
    }

    private static string? Run(string directory, string fileName, bool useHead)
    {
        try
        {
            var startInfo = new ProcessStartInfo
            {
                FileName = "git",
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                StandardOutputEncoding = Encoding.UTF8,
                StandardErrorEncoding = Encoding.UTF8
            };

            startInfo.ArgumentList.Add("-C");
            startInfo.ArgumentList.Add(directory);
            startInfo.ArgumentList.Add("diff");
            startInfo.ArgumentList.Add("--no-color");
            startInfo.ArgumentList.Add("--no-ext-diff");
            startInfo.ArgumentList.Add("--no-textconv");
            startInfo.ArgumentList.Add("--unified=0");
            if (useHead)
            {
                startInfo.ArgumentList.Add("HEAD");
            }
            startInfo.ArgumentList.Add("--");
            startInfo.ArgumentList.Add(":(literal)" + fileName);

            using Process? process = Process.Start(startInfo);
            if (process is null)
            {
                return null;
            }

            process.StandardInput.Close();
            Task<string> outputTask = process.StandardOutput.ReadToEndAsync();
            Task<string> errorTask = process.StandardError.ReadToEndAsync();

            if (!process.WaitForExit(HelperTimeoutMilliseconds))
            {
                try
                {
                    process.Kill(entireProcessTree: true);
                }
                catch (Exception ex) when (ex is InvalidOperationException or
                                           NotSupportedException or
                                           System.ComponentModel.Win32Exception)
                {
                    // The process may already have exited.
                }

                return null;
            }

            string output = outputTask.GetAwaiter().GetResult();
            _ = errorTask.GetAwaiter().GetResult();
            return process.ExitCode == 0 ? output : null;
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or
                                   IOException or InvalidOperationException or
                                   NotSupportedException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    private static void Parse(string output, long lineCount,
        Dictionary<long, GitDiffLineKind> result)
    {
        foreach (string line in output.Split('\n'))
        {
            Match match = HunkHeader.Match(line);
            if (!match.Success)
            {
                continue;
            }

            long oldCount = ReadCount(match, "oldCount");
            long newStart = long.Parse(match.Groups["newStart"].Value,
                CultureInfo.InvariantCulture);
            long newCount = ReadCount(match, "newCount");

            long start = Math.Max(1, newStart);
            long first = start - 1;
            long common = Math.Min(oldCount, newCount);

            for (long index = 0; index < common; ++index)
            {
                result[first + index] = GitDiffLineKind.Modified;
            }

            for (long index = common; index < newCount; ++index)
            {
                result[first + index] = GitDiffLineKind.Added;
            }

            if (oldCount > newCount)
            {
                // A zero-length hunk names the line before the deletion point.
                long deletedLine = newCount == 0 ? newStart : first + newCount;
                result[Math.Clamp(deletedLine, 0, lineCount - 1)] =
                    GitDiffLineKind.Deleted;
            }
        }
    }

    private static long ReadCount(Match match, string group)
        => match.Groups[group].Success
            ? long.Parse(match.Groups[group].Value, CultureInfo.InvariantCulture)
            : 1;
}

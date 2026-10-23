using System.Text;

namespace Hexal.Util;

/// <summary>
/// Builds a unified diff between two text documents. The implementation uses
/// the same Myers line comparison as the Git diff gutter helper and keeps
/// three lines of context around each change.
/// </summary>
public static class UnifiedDiff
{
    private const int ContextLines = 3;
    private const int MaxDepth = 2_000;
    private const long MemoryBudget = 32L * 1024 * 1024;

    private enum EditKind : byte
    {
        Equal,
        Delete,
        Insert,
    }

    private readonly record struct Operation(
        EditKind Kind,
        int OldIndex,
        int NewIndex,
        string Text
    );

    public static string Create(string oldLabel, string oldText, string newLabel, string newText)
    {
        string[] oldLines = SplitLines(oldText);
        string[] newLines = SplitLines(newText);
        ulong[] oldHashes = oldLines.Select(HashLine).ToArray();
        ulong[] newHashes = newLines.Select(HashLine).ToArray();

        List<EditKind>? edits = Myers(oldHashes, newHashes);
        if (edits is null)
        {
            edits = new List<EditKind>(oldLines.Length + newLines.Length);
            for (int index = 0; index < oldLines.Length; ++index)
            {
                edits.Add(EditKind.Delete);
            }
            for (int index = 0; index < newLines.Length; ++index)
            {
                edits.Add(EditKind.Insert);
            }
        }

        var operations = new List<Operation>(edits.Count);
        int oldIndex = 0;
        int newIndex = 0;
        foreach (EditKind edit in edits)
        {
            switch (edit)
            {
                case EditKind.Equal:
                    operations.Add(new Operation(edit, oldIndex, newIndex, oldLines[oldIndex]));
                    ++oldIndex;
                    ++newIndex;
                    break;
                case EditKind.Delete:
                    operations.Add(new Operation(edit, oldIndex, newIndex, oldLines[oldIndex]));
                    ++oldIndex;
                    break;
                case EditKind.Insert:
                    operations.Add(new Operation(edit, oldIndex, newIndex, newLines[newIndex]));
                    ++newIndex;
                    break;
            }
        }

        var output = new StringBuilder();
        output.Append("--- ").Append(oldLabel).Append('\n');
        output.Append("+++ ").Append(newLabel).Append('\n');
        AppendHunks(output, operations);
        return output.ToString();
    }

    private static string[] SplitLines(string text)
    {
        if (text.Length == 0)
        {
            return Array.Empty<string>();
        }

        var lines = new List<string>();
        int start = 0;
        for (int index = 0; index < text.Length; ++index)
        {
            if (text[index] != '\n')
            {
                continue;
            }

            int end = index;
            if (end > start && text[end - 1] == '\r')
            {
                --end;
            }

            lines.Add(text[start..end]);
            start = index + 1;
        }

        if (start < text.Length)
        {
            int end = text.Length;
            if (end > start && text[end - 1] == '\r')
            {
                --end;
            }
            lines.Add(text[start..end]);
        }

        return lines.ToArray();
    }

    private static ulong HashLine(string line)
    {
        ulong hash = 14695981039346656037UL;
        foreach (char character in line)
        {
            hash ^= character;
            hash *= 1099511628211UL;
        }
        return hash;
    }

    private static void AppendHunks(StringBuilder output, List<Operation> operations)
    {
        if (operations.Count == 0)
        {
            return;
        }

        var oldAt = new int[operations.Count];
        var newAt = new int[operations.Count];
        int oldLine = 0;
        int newLine = 0;
        for (int index = 0; index < operations.Count; ++index)
        {
            oldAt[index] = oldLine;
            newAt[index] = newLine;
            switch (operations[index].Kind)
            {
                case EditKind.Equal:
                    ++oldLine;
                    ++newLine;
                    break;
                case EditKind.Delete:
                    ++oldLine;
                    break;
                case EditKind.Insert:
                    ++newLine;
                    break;
            }
        }

        var changes = new List<int>();
        for (int index = 0; index < operations.Count; ++index)
        {
            if (operations[index].Kind != EditKind.Equal)
            {
                changes.Add(index);
            }
        }

        if (changes.Count == 0)
        {
            return;
        }

        var hunks = new List<(int Start, int End)>();
        int start = Math.Max(0, changes[0] - ContextLines);
        int end = Math.Min(operations.Count - 1, changes[0] + ContextLines);
        for (int index = 1; index < changes.Count; ++index)
        {
            int nextStart = Math.Max(0, changes[index] - ContextLines);
            int nextEnd = Math.Min(operations.Count - 1, changes[index] + ContextLines);
            if (nextStart <= end + 1)
            {
                end = Math.Max(end, nextEnd);
            }
            else
            {
                hunks.Add((start, end));
                start = nextStart;
                end = nextEnd;
            }
        }
        hunks.Add((start, end));

        foreach ((int hunkStart, int hunkEnd) in hunks)
        {
            int oldCount = 0;
            int newCount = 0;
            for (int index = hunkStart; index <= hunkEnd; ++index)
            {
                switch (operations[index].Kind)
                {
                    case EditKind.Equal:
                        ++oldCount;
                        ++newCount;
                        break;
                    case EditKind.Delete:
                        ++oldCount;
                        break;
                    case EditKind.Insert:
                        ++newCount;
                        break;
                }
            }

            int oldStart = oldAt[hunkStart] + 1;
            int newStart = newAt[hunkStart] + 1;
            output
                .Append("@@ -")
                .Append(oldStart)
                .Append(',')
                .Append(oldCount)
                .Append(" +")
                .Append(newStart)
                .Append(',')
                .Append(newCount)
                .Append(" @@\n");

            for (int index = hunkStart; index <= hunkEnd; ++index)
            {
                Operation operation = operations[index];
                output.Append(
                    operation.Kind switch
                    {
                        EditKind.Delete => '-',
                        EditKind.Insert => '+',
                        _ => ' ',
                    }
                );
                output.Append(operation.Text).Append('\n');
            }
        }
    }

    private static List<EditKind>? Myers(ulong[] oldLines, ulong[] newLines)
    {
        int oldCount = oldLines.Length;
        int newCount = newLines.Length;
        int max = oldCount + newCount;
        int offset = max;
        var frontier = new int[max * 2 + 1];
        long frontierBytes = (long)frontier.Length * sizeof(int);
        int depthBudget = (int)
            Math.Min(max, Math.Min(MaxDepth, MemoryBudget / Math.Max(1L, frontierBytes)));
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
                if (
                    diagonal == -depth
                    || (
                        diagonal != depth
                        && frontier[offset + diagonal - 1] < frontier[offset + diagonal + 1]
                    )
                )
                {
                    x = frontier[offset + diagonal + 1];
                }
                else
                {
                    x = frontier[offset + diagonal - 1] + 1;
                }

                int y = x - diagonal;
                while (x < oldCount && y < newCount && oldLines[x] == newLines[y])
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

    private static List<EditKind> Backtrack(
        List<int[]> trace,
        int depth,
        int offset,
        int oldCount,
        int newCount
    )
    {
        var edits = new List<EditKind>(oldCount + newCount);
        int x = oldCount;
        int y = newCount;

        for (int current = depth; current >= 0; --current)
        {
            int[] frontier = trace[current];
            int diagonal = x - y;
            int previousDiagonal;
            if (
                diagonal == -current
                || (
                    diagonal != current
                    && frontier[offset + diagonal - 1] < frontier[offset + diagonal + 1]
                )
            )
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
}

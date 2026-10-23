namespace Hexal.Languages;

/// <summary>
/// Renders the <c>--health</c> report in a compact, fixed-width table. The
/// layout mirrors Helix's health table without the debug adapter, textobject
/// and indent columns.
/// </summary>
public static class LanguageHealthTable
{
    private const int LanguageWidth = 14;
    private const int ServerWidth = 32;
    private const int FormatterWidth = 18;

    public static void Write(IReadOnlyList<LanguageHealthEntry> entries,
        TextWriter writer)
    {
        WriteRow(writer, "Language", "Language server", "Formatter", "Highlight");
        foreach (LanguageHealthEntry entry in entries)
        {
            WriteRow(writer, entry.Language, ServerCell(entry),
                FormatterCell(entry), entry.Highlighting ? "✓" : "✘");
        }
    }

    private static string ServerCell(LanguageHealthEntry entry)
    {
        if (entry.LanguageServer is null)
        {
            return "None";
        }

        return $"{(entry.LanguageServerAvailable ? "✓" : "✘")} {entry.LanguageServer}";
    }

    private static string FormatterCell(LanguageHealthEntry entry)
    {
        if (entry.Formatter is null)
        {
            return "None";
        }

        return $"{(entry.FormatterAvailable ? "✓" : "✘")} {entry.Formatter}";
    }

    private static void WriteRow(TextWriter writer, string language, string server,
        string formatter, string highlight)
    {
        writer.WriteLine(
            $"{Cell(language, LanguageWidth)} " +
            $"{Cell(server, ServerWidth)} " +
            $"{Cell(formatter, FormatterWidth)} " +
            highlight);
    }

    private static string Cell(string value, int width)
    {
        if (value.Length > width)
        {
            value = width <= 1 ? "…" : value[..(width - 1)] + "…";
        }

        return value.PadRight(width);
    }
}

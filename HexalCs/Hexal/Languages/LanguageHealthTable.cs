namespace Hexal.Languages;

/// <summary>
/// Renders the <c>--health</c> report in a compact, fixed-width table. The
/// layout mirrors Helix's health table with language server, formatter,
/// debug-adapter and highlighting columns.
/// </summary>
public static class LanguageHealthTable
{
    private const int LanguageWidth = 14;
    private const int ServerWidth = 32;
    private const int FormatterWidth = 18;
    private const int DebugAdapterWidth = 18;

    private const string Reset = "\x1b[0m";
    private const string White = "\x1b[37m";
    private const string Header = "\x1b[1;37m";
    private const string Cyan = "\x1b[36m";
    private const string Lime = "\x1b[38;5;118m";
    private const string Red = "\x1b[31m";

    public static void Write(
        IReadOnlyList<LanguageHealthEntry> entries,
        TextWriter writer,
        bool useColor = false
    )
    {
        WriteCell(writer, "Language", LanguageWidth, Header, useColor);
        writer.Write(' ');
        WriteCell(writer, "Language server", ServerWidth, Header, useColor);
        writer.Write(' ');
        WriteCell(writer, "Formatter", FormatterWidth, Header, useColor);
        writer.Write(' ');
        WriteCell(writer, "Debug adapter", DebugAdapterWidth, Header, useColor);
        writer.Write(' ');
        WriteText(writer, "Highlight", Header, useColor);
        writer.WriteLine();

        foreach (LanguageHealthEntry entry in entries)
        {
            WriteCell(writer, entry.Language, LanguageWidth, White, useColor);
            writer.Write(' ');
            WriteStatusCell(
                writer,
                entry.LanguageServer,
                entry.LanguageServerAvailable,
                ServerWidth,
                useColor
            );
            writer.Write(' ');
            WriteStatusCell(
                writer,
                entry.Formatter,
                entry.FormatterAvailable,
                FormatterWidth,
                useColor
            );
            writer.Write(' ');
            WriteStatusCell(
                writer,
                entry.DebugAdapter,
                entry.DebugAdapterAvailable,
                DebugAdapterWidth,
                useColor
            );
            writer.Write(' ');
            WriteText(
                writer,
                entry.Highlighting ? "✓" : "✘",
                entry.Highlighting ? Lime : Red,
                useColor
            );
            writer.WriteLine();
        }
    }

    private static void WriteStatusCell(
        TextWriter writer,
        string? value,
        bool available,
        int width,
        bool useColor
    )
    {
        if (value is null)
        {
            WriteCell(writer, "None", width, Cyan, useColor);
            return;
        }

        string text = $"{(available ? "✓" : "✘")} {value}";
        WriteCell(writer, text, width, available ? Lime : Red, useColor);
    }

    private static void WriteCell(
        TextWriter writer,
        string value,
        int width,
        string color,
        bool useColor
    )
    {
        string cell = Cell(value, width);
        string text = cell.TrimEnd();
        WriteText(writer, text, color, useColor);
        writer.Write(new string(' ', width - text.Length));
    }

    private static void WriteText(TextWriter writer, string text, string color, bool useColor)
    {
        if (useColor)
        {
            writer.Write(color);
        }
        writer.Write(text);
        if (useColor)
        {
            writer.Write(Reset);
        }
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

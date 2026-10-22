using System.Text;
using System.Text.Json;
using Hexal.Util;

namespace Hexal.Core;

public sealed class HexalConfig
{
    public const string DefaultFileName = ".hexcfg";

    private const string DefaultContents = """
        {
            "version": 1,
            "formatOnSave": false
        }

        """;

    public string SourcePath { get; init; } = string.Empty;

    public bool FormatOnSave { get; init; }

    public static string DefaultPath =>
        Path.Combine(HexalPaths.HexalHomeDirectory, DefaultFileName);

    public static string AlternatePath =>
        Path.Combine(HexalPaths.HexalHomeDirectory, "hexcfg");

    public static string ResolvePath()
    {
        string primary = DefaultPath;
        if (File.Exists(primary))
        {
            return primary;
        }

        string alternate = AlternatePath;
        return File.Exists(alternate) ? alternate : primary;
    }

    public static HexalConfig LoadDefault(out string error)
    {
        string path = ResolvePath();
        if (!File.Exists(path))
        {
            error = string.Empty;
            return new HexalConfig { SourcePath = path };
        }

        return Load(path, out error);
    }

    public static HexalConfig Load(string path, out string error)
    {
        error = string.Empty;

        try
        {
            string json = File.ReadAllText(path);
            if (!TryParse(json, path, out HexalConfig? config, out error))
            {
                return new HexalConfig { SourcePath = path };
            }

            return config!;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            error = ex.Message;
            return new HexalConfig { SourcePath = path };
        }
    }

    public static bool TryParse(string json, string sourcePath,
        out HexalConfig? config, out string error)
    {
        config = null;
        error = string.Empty;

        try
        {
            using JsonDocument document = JsonDocument.Parse(json,
                new JsonDocumentOptions
                {
                    AllowTrailingCommas = true,
                    CommentHandling = JsonCommentHandling.Skip
                });

            JsonElement root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                error = "root must be a JSON object";
                return false;
            }

            bool? formatOnSave = ReadBool(root, "formatOnSave") ??
                                 ReadBool(root, "format_on_save");

            if (formatOnSave is null &&
                root.TryGetProperty("editor", out JsonElement editor) &&
                editor.ValueKind == JsonValueKind.Object)
            {
                formatOnSave = ReadBool(editor, "formatOnSave") ??
                               ReadBool(editor, "format_on_save");
            }

            config = new HexalConfig
            {
                SourcePath = sourcePath,
                FormatOnSave = formatOnSave ?? false
            };
            return true;
        }
        catch (JsonException ex)
        {
            error = $"invalid JSON: {ex.Message}";
            return false;
        }
    }

    public static bool EnsureDefaultFile(out string path, out string error)
    {
        path = ResolvePath();
        error = string.Empty;

        try
        {
            Directory.CreateDirectory(HexalPaths.HexalHomeDirectory);
            if (!File.Exists(path))
            {
                File.WriteAllText(path, DefaultContents, new UTF8Encoding(false));
            }
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            error = ex.Message;
            return false;
        }
    }

    private static bool? ReadBool(JsonElement element, string property)
    {
        if (!element.TryGetProperty(property, out JsonElement value))
        {
            return null;
        }

        return value.ValueKind switch
        {
            JsonValueKind.True => true,
            JsonValueKind.False => false,
            _ => null
        };
    }
}

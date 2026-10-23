using System.Globalization;
using System.Text;
using System.Text.Json.Nodes;

namespace Hexal.Lsp;

/// <summary>
/// URI helpers used by the Lua LSP modules. File URIs are normalized, but the
/// runtime never assumes that every URI is file-backed (virtual documents and
/// custom schemes are supported).
/// </summary>
public static class LspUri
{
    public static string FromPath(string path)
    {
        if (string.IsNullOrWhiteSpace(path))
        {
            return string.Empty;
        }

        try
        {
            string full = Path.GetFullPath(path);
            return new Uri(full).AbsoluteUri;
        }
        catch (Exception ex)
            when (ex
                    is ArgumentException
                        or NotSupportedException
                        or PathTooLongException
                        or UriFormatException
            )
        {
            return path.Replace('\\', '/');
        }
    }

    public static string ToPath(string uri)
    {
        if (string.IsNullOrWhiteSpace(uri))
        {
            return string.Empty;
        }

        try
        {
            var parsed = new Uri(uri, UriKind.Absolute);
            return parsed.IsFile ? parsed.LocalPath : parsed.AbsoluteUri;
        }
        catch (Exception ex)
            when (ex is UriFormatException or ArgumentException or NotSupportedException)
        {
            return uri;
        }
    }

    public static string Normalize(string uri)
    {
        if (string.IsNullOrWhiteSpace(uri))
        {
            return string.Empty;
        }

        try
        {
            var parsed = new Uri(uri, UriKind.Absolute);
            if (parsed.IsFile)
            {
                return new Uri(Path.GetFullPath(parsed.LocalPath)).AbsoluteUri;
            }
            return parsed.AbsoluteUri;
        }
        catch (Exception ex)
            when (ex
                    is UriFormatException
                        or ArgumentException
                        or NotSupportedException
                        or PathTooLongException
            )
        {
            return uri.Replace('\\', '/');
        }
    }

    public static bool IsFile(string uri)
    {
        try
        {
            return new Uri(uri, UriKind.Absolute).IsFile;
        }
        catch (Exception ex) when (ex is UriFormatException or ArgumentException)
        {
            return false;
        }
    }

    public static JsonObject Parse(string uri)
    {
        var result = new JsonObject { ["uri"] = uri };

        try
        {
            var parsed = new Uri(uri, UriKind.Absolute);
            result["scheme"] = parsed.Scheme;
            result["authority"] = parsed.Authority;
            result["path"] = parsed.IsFile ? parsed.LocalPath : parsed.AbsolutePath;
            result["query"] = parsed.Query.TrimStart('?');
            result["fragment"] = parsed.Fragment.TrimStart('#');
            result["file"] = parsed.IsFile;
        }
        catch (Exception ex)
            when (ex is UriFormatException or ArgumentException or NotSupportedException)
        {
            result["scheme"] = string.Empty;
            result["path"] = uri;
            result["file"] = false;
        }

        return result;
    }

    public static string FileName(string path) => Path.GetFileName(path);

    public static bool PathsEqual(string left, string right)
    {
        try
        {
            return string.Equals(
                Path.GetFullPath(left),
                Path.GetFullPath(right),
                OperatingSystem.IsWindows()
                    ? StringComparison.OrdinalIgnoreCase
                    : StringComparison.Ordinal
            );
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return string.Equals(left, right, StringComparison.Ordinal);
        }
    }

    public static bool IsWithin(string path, string directory)
    {
        try
        {
            string fullPath = Path.GetFullPath(path);
            string fullDirectory = Path.GetFullPath(directory);
            if (!fullDirectory.EndsWith(Path.DirectorySeparatorChar))
            {
                fullDirectory += Path.DirectorySeparatorChar;
            }

            return fullPath.StartsWith(
                    fullDirectory,
                    OperatingSystem.IsWindows()
                        ? StringComparison.OrdinalIgnoreCase
                        : StringComparison.Ordinal
                )
                || string.Equals(
                    fullPath,
                    fullDirectory.TrimEnd(Path.DirectorySeparatorChar),
                    OperatingSystem.IsWindows()
                        ? StringComparison.OrdinalIgnoreCase
                        : StringComparison.Ordinal
                );
        }
        catch (Exception ex)
            when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return false;
        }
    }

    public static string DecodePercent(string value)
    {
        return Uri.UnescapeDataString(value);
    }

    public static string EncodePercent(string value)
    {
        return Uri.EscapeDataString(value);
    }

    public static string Combine(string root, string relative)
    {
        if (Path.IsPathRooted(relative))
        {
            return relative;
        }
        return Path.Combine(root, relative);
    }

    public static string Describe(JsonObject parsed)
    {
        var builder = new StringBuilder();
        builder.Append(LspJson.GetString(parsed["scheme"]) ?? string.Empty);
        builder.Append("://");
        builder.Append(LspJson.GetString(parsed["authority"]) ?? string.Empty);
        builder.Append(LspJson.GetString(parsed["path"]) ?? string.Empty);
        return builder.ToString();
    }

    public static string GlobToRegex(string glob)
    {
        var builder = new StringBuilder("^");
        for (int index = 0; index < glob.Length; ++index)
        {
            char character = glob[index];
            switch (character)
            {
                case '*':
                    if (index + 1 < glob.Length && glob[index + 1] == '*')
                    {
                        builder.Append(".*");
                        ++index;
                    }
                    else
                    {
                        builder.Append("[^/\\\\]*");
                    }
                    break;
                case '?':
                    builder.Append('.');
                    break;
                case '/':
                case '\\':
                    builder.Append("[/\\\\]");
                    break;
                case '.':
                case '+':
                case '(':
                case ')':
                case '[':
                case ']':
                case '{':
                case '}':
                case '^':
                case '$':
                case '|':
                    builder.Append('\\').Append(character);
                    break;
                default:
                    builder.Append(character);
                    break;
            }
        }
        builder.Append('$');
        return builder.ToString();
    }

    public static string FormatInvariant(long value) =>
        value.ToString(CultureInfo.InvariantCulture);
}

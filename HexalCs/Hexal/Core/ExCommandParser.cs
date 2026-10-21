namespace Hexal.Core;

public sealed class ExCommand
{
    public string Name { get; set; } = string.Empty;
    public List<string> Arguments { get; } = new();
}

public sealed class ExParseResult
{
    public bool Ok { get; set; }
    public ExCommand Command { get; set; } = new();
    public string Error { get; set; } = string.Empty;
}

/// <summary>
/// Parses the text after the leading ':' of an Ex command. Filenames with
/// spaces may be single- or double-quoted. Inside double quotes only \" is an
/// escape; every other backslash is literal so Windows and UNC paths survive.
/// </summary>
public static class ExCommandParser
{
    public static ExParseResult Parse(ReadOnlySpan<char> input)
    {
        var result = new ExParseResult { Ok = true };
        int pos = 0;

        SkipSpace(input, ref pos);
        if (pos >= input.Length)
        {
            return result;
        }

        int nameStart = pos;
        while (pos < input.Length && !IsSpace(input[pos]))
        {
            ++pos;
        }
        result.Command.Name = input[nameStart..pos].ToString();

        while (true)
        {
            SkipSpace(input, ref pos);
            if (pos >= input.Length)
            {
                break;
            }

            char first = input[pos];
            if (first is '"' or '\'')
            {
                char quote = first;
                ++pos;
                var argument = new System.Text.StringBuilder();

                while (pos < input.Length && input[pos] != quote)
                {
                    if (quote == '"' && input[pos] == '\\' && pos + 1 < input.Length &&
                        input[pos + 1] == '"')
                    {
                        argument.Append('"');
                        pos += 2;
                        continue;
                    }

                    argument.Append(input[pos]);
                    ++pos;
                }

                if (pos >= input.Length)
                {
                    result.Ok = false;
                    result.Error = quote == '"'
                        ? "unterminated double quote"
                        : "unterminated single quote";
                    result.Command.Arguments.Clear();
                    return result;
                }

                ++pos; // closing quote
                result.Command.Arguments.Add(argument.ToString());
                continue;
            }

            int argumentStart = pos;
            while (pos < input.Length && !IsSpace(input[pos]))
            {
                ++pos;
            }
            result.Command.Arguments.Add(input[argumentStart..pos].ToString());
        }

        return result;
    }

    private static void SkipSpace(ReadOnlySpan<char> input, ref int pos)
    {
        while (pos < input.Length && IsSpace(input[pos]))
        {
            ++pos;
        }
    }

    private static bool IsSpace(char c)
        => c is ' ' or '\t' or '\n' or '\r' or '\v' or '\f';
}

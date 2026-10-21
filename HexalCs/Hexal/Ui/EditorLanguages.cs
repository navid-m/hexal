using Hexal.Languages;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private void RefreshLanguage()
    {
        _language = _document.Path is null ? null : _languages.Detect(_document.Path);
        _syntax.SetDocument(_document, _language);
    }

    private void SetLanguage(LanguageDefinition? language)
    {
        _language = language;
        _syntax.SetDocument(_document, language);
    }

    private bool TrySetLanguage(string id, out string error)
    {
        error = string.Empty;
        LanguageDefinition? language = _languages.Find(id);
        if (language is null)
        {
            error = $"unknown language '{id}'";
            return false;
        }

        SetLanguage(language);
        return true;
    }

    private static Style TokenStyle(SyntaxTokenKind kind)
    {
        return kind switch
        {
            SyntaxTokenKind.Keyword => new Style { Fg = 13 },
            SyntaxTokenKind.ControlStatement => new Style { Fg = 12 },
            SyntaxTokenKind.Type => new Style { Fg = 14 },
            SyntaxTokenKind.Constant => new Style { Fg = 11 },
            SyntaxTokenKind.StringLiteral => new Style { Fg = 11 },
            SyntaxTokenKind.Comment => new Style { Fg = 10 },
            SyntaxTokenKind.Number => new Style { Fg = 11 },
            _ => new Style()
        };
    }
}

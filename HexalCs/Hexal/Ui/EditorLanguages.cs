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

    private Style TokenStyle(SyntaxTokenKind kind)
    {
        return kind switch
        {
            SyntaxTokenKind.Keyword => _theme.SyntaxStyle(_theme.Keyword),
            SyntaxTokenKind.ControlStatement => _theme.SyntaxStyle(_theme.ControlStatement),
            SyntaxTokenKind.Type => _theme.SyntaxStyle(_theme.Type),
            SyntaxTokenKind.Constant => _theme.SyntaxStyle(_theme.Constant),
            SyntaxTokenKind.StringLiteral => _theme.SyntaxStyle(_theme.StringLiteral),
            SyntaxTokenKind.Comment => _theme.SyntaxStyle(_theme.Comment),
            SyntaxTokenKind.Number => _theme.SyntaxStyle(_theme.Number),
            _ => _theme.TextStyle
        };
    }
}

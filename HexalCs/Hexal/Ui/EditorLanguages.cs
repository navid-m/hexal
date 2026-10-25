using Hexal.Languages;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private void RefreshLanguage()
    {
        string? languageId = _document.VirtualLanguageId;
        SetLanguage(
            languageId is not null ? _languages.Find(languageId)
            : _document.Path is null ? null
            : _languages.Detect(_document.Path)
        );
    }

    private void SetLanguage(LanguageDefinition? language)
    {
        _language = language;
        ApplyLanguageIndentation(language);
        _syntax.SetDocument(_document, language);
    }

    /// <summary>
    /// Applies the default indentation preferences declared by a grammar.
    /// Languages without an <c>indentation</c> object leave the buffer's
    /// current settings alone.
    /// </summary>
    private void ApplyLanguageIndentation(LanguageDefinition? language)
    {
        IndentationSettings? indentation = language?.Indentation;
        if (indentation is null)
        {
            return;
        }

        IndentationSettings settings = indentation.Value;
        if (settings.UseTabs)
        {
            _document.SetIndentTabs(settings.TabWidth);
            _document.IndentWidth = settings.IndentWidth;
        }
        else
        {
            _document.TabWidth = settings.TabWidth;
            _document.SetIndentSpaces(settings.IndentWidth);
        }
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
            SyntaxTokenKind.Added => _theme.SyntaxStyle(_theme.DiffAdded),
            SyntaxTokenKind.Deleted => _theme.SyntaxStyle(_theme.DiffDeleted),
            _ => _theme.TextStyle,
        };
    }
}

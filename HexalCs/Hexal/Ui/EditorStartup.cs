using System.Diagnostics;
using Hexal.Languages;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private readonly long _editorStartTimestamp = Stopwatch.GetTimestamp();
    private Task<LanguageRegistry>? _startupGrammarTask;
    private long _startupGrammarStarted;
    private bool _postRenderStartupDone;

    /// <summary>
    /// Completes work that is not needed for the first frame. This is polled
    /// after the initial render so a slow disk or a large grammar set cannot
    /// leave the terminal showing an empty screen.
    /// </summary>
    private bool PollStartupWork()
    {
        bool redraw = false;

        if (!_postRenderStartupDone)
        {
            _postRenderStartupDone = true;
            long postRenderStarted = _benchmark is null
                ? 0
                : Stopwatch.GetTimestamp();
            AutoDetectIndentationOnOpen();
            RefreshDiffLines();
            _benchmark?.AddSince("Startup", "Post-render initialization",
                postRenderStarted);
            redraw = true;
        }

        if (_startupGrammarTask is not { IsCompleted: true } completed)
        {
            return redraw;
        }

        _startupGrammarTask = null;
        LanguageRegistry languages;
        try
        {
            languages = completed.GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            languages = new LanguageRegistry();
            SetError($"grammar: {ex.Message}");
        }

        _languages = languages;
        if (_benchmark is not null && _startupGrammarStarted != 0)
        {
            _benchmark.AddSince("Internal", "Grammar registry",
                _startupGrammarStarted,
                $"{_languages.Languages.Count} definitions");
            _startupGrammarStarted = 0;
        }

        RefreshLanguage();
        // Language detection can change the default indentation. Preserve the
        // original startup order where document auto-detection wins.
        AutoDetectIndentationOnOpen();

        if (_languages.Errors.Count > 0)
        {
            SetError($"grammar: {_languages.Errors[0]}");
        }

        // Language servers and lazy language plugins need the detected
        // language, which was not available for the first render.
        FirePluginEvent("buffer_opened", CurrentBufferData());

        _forceFullRedraw = true;
        return true;
    }
}

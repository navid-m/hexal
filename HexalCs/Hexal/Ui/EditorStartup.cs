using System.Diagnostics;
using Hexal.Languages;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private readonly long _editorStartTimestamp = Stopwatch.GetTimestamp();
    private Task<LanguageRegistry>? _startupGrammarTask;
    private long _startupGrammarStarted;
    private bool _postRenderStartupDone;
    private bool _sidebarPrewarmed;

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
            long postRenderStarted = _benchmark is null ? 0 : Stopwatch.GetTimestamp();
            long phaseStarted = StartupTrace.Timestamp();
            AutoDetectIndentationOnOpen();
            StartupTrace.Mark("post-render indentation detection", phaseStarted);

            phaseStarted = StartupTrace.Timestamp();
            RefreshDiffLines();
            StartupTrace.Mark(
                "post-render git diff lines",
                phaseStarted,
                $"{_diffLines.Count} marker(s)"
            );
            _benchmark?.AddSince("Startup", "Post-render initialization", postRenderStarted);
            redraw = true;
        }

        if (!_sidebarPrewarmed)
        {
            _sidebarPrewarmed = true;
            ExpandSidebarToPath(_document.Path);
            RefreshSidebar(_document.Path);
        }

        if (_startupGrammarTask is not { IsCompleted: true } completed)
        {
            return redraw;
        }

        long grammarCompleted = StartupTrace.Timestamp();
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
        StartupTrace.Mark("collect grammar task result", grammarCompleted);

        _languages = languages;
        if (_benchmark is not null && _startupGrammarStarted != 0)
        {
            _benchmark.AddSince(
                "Internal",
                "Grammar registry",
                _startupGrammarStarted,
                $"{_languages.Languages.Count} definitions"
            );
            _startupGrammarStarted = 0;
        }
        StartupTrace.Note(
            $"grammar registry applied: {_languages.Languages.Count} definitions, "
                + $"{_languages.Errors.Count} error(s)"
        );

        long phaseStarted2 = StartupTrace.Timestamp();
        RefreshLanguage();
        StartupTrace.Mark(
            "post-grammar language detection",
            phaseStarted2,
            _language?.Id ?? "<none>"
        );

        // Language detection can change the default indentation. Preserve the
        // original startup order where document auto-detection wins.
        phaseStarted2 = StartupTrace.Timestamp();
        AutoDetectIndentationOnOpen();
        StartupTrace.Mark("post-grammar indentation detection", phaseStarted2);

        if (_languages.Errors.Count > 0)
        {
            SetError($"grammar: {_languages.Errors[0]}");
        }

        // Language servers and lazy language plugins need the detected
        // language, which was not available for the first render.
        phaseStarted2 = StartupTrace.Timestamp();
        FirePluginEvent("buffer_opened", CurrentBufferData());
        StartupTrace.Mark(
            "post-grammar buffer_opened event",
            phaseStarted2,
            $"language={_language?.Id ?? "<none>"}"
        );
        StartupTrace.DumpBenchmark(_benchmark, "after grammar/deferred plugins");
        StartupTrace.Note(
            $"startup work fully drained: main+{StartupTrace.SinceMainMilliseconds:F1} ms, "
                + $"process+{StartupTrace.SinceProcessMilliseconds():F1} ms"
        );

        _forceFullRedraw = true;
        return true;
    }
}

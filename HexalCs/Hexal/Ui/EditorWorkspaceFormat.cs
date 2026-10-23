using Hexal.Core;
using Hexal.Languages;
using Hexal.Util;

namespace Hexal.Ui;

public sealed partial class Editor
{
    private sealed class WorkspaceFormatOperation : IDisposable
    {
        public Task<WorkspaceFormatResult> Task = null!;
        public CancellationTokenSource Cancellation = new();
        public string Extension = string.Empty;
        public string CurrentFile = string.Empty;
        public int Total;
        public int Completed;

        public void Dispose() => Cancellation.Dispose();
    }

    private sealed class WorkspaceFormatResult
    {
        public int Matched;
        public int Formatted;
        public int Changed;
        public int Failed;
        public int Skipped;
        public int SkippedOpen;
        public string FirstError = string.Empty;
    }

    private WorkspaceFormatOperation? _workspaceFormatOperation;
    private int _workspaceFormatLastTotal = -1;
    private int _workspaceFormatLastCompleted = -1;
    private string _workspaceFormatLastFile = string.Empty;

    private void FormatWorkspaceExtension(string argument)
    {
        if (
            _formatOperation is not null
            || _sidebarFormatOperation is not null
            || _workspaceFormatOperation is not null
        )
        {
            SetMessage("Formatting is already in progress.");
            return;
        }

        if (!TryNormalizeExtension(argument, out string extension))
        {
            SetError("usage: :fmta <extension>");
            return;
        }

        LanguageDefinition? language = _languages.Detect("file." + extension);
        if (language is null)
        {
            SetError($"No language is configured for .{extension} files.");
            return;
        }

        if (language.Formatter is null)
        {
            SetError($"No formatter configured for {language.Name}.");
            return;
        }

        string root = ProjectRoot();
        string openPath = _document.Path ?? string.Empty;
        var operation = new WorkspaceFormatOperation { Extension = extension };
        operation.Task = Task.Factory.StartNew(
            () => RunWorkspaceFormat(root, extension, _languages, openPath, operation),
            CancellationToken.None,
            TaskCreationOptions.LongRunning,
            TaskScheduler.Default
        );

        _workspaceFormatOperation = operation;
        _workspaceFormatLastTotal = -1;
        _workspaceFormatLastCompleted = -1;
        _workspaceFormatLastFile = string.Empty;
        SetMessage($"Formatting .{extension} files in the workspace...");
    }

    private static WorkspaceFormatResult RunWorkspaceFormat(
        string root,
        string extension,
        LanguageRegistry languages,
        string openPath,
        WorkspaceFormatOperation operation
    )
    {
        var result = new WorkspaceFormatResult();
        CancellationToken token = operation.Cancellation.Token;
        string suffix = "." + extension;

        List<string> files;
        try
        {
            files = FileSearch.EnumerateFiles(root, int.MaxValue, showIgnored: false, token);
        }
        catch (OperationCanceledException)
        {
            throw;
        }
        catch (Exception ex)
            when (ex
                    is IOException
                        or UnauthorizedAccessException
                        or NotSupportedException
                        or ArgumentException
            )
        {
            ++result.Failed;
            result.FirstError = ex.Message;
            return result;
        }

        var jobs = new List<(string Path, FormatterConfig Formatter)>();
        foreach (string relative in files)
        {
            token.ThrowIfCancellationRequested();
            if (!relative.EndsWith(suffix, StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            ++result.Matched;
            string path;
            try
            {
                path = Path.Combine(root, relative);
            }
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                ++result.Skipped;
                result.FirstError ??= ex.Message;
                continue;
            }

            LanguageDefinition? language = languages.Detect(path);
            if (language?.Formatter is null)
            {
                ++result.Skipped;
                continue;
            }

            jobs.Add((path, language.Formatter));
        }

        Volatile.Write(ref operation.Total, jobs.Count);
        foreach ((string path, FormatterConfig formatter) in jobs)
        {
            if (token.IsCancellationRequested)
            {
                break;
            }

            Volatile.Write(ref operation.CurrentFile, Path.GetFileName(path));
            if (openPath.Length > 0 && SamePath(path, openPath))
            {
                ++result.SkippedOpen;
                Interlocked.Increment(ref operation.Completed);
                continue;
            }

            try
            {
                byte[] input = File.ReadAllBytes(path);
                FormatResult formatted = FormatterRunner.Run(formatter, input, path, token);
                if (token.IsCancellationRequested)
                {
                    break;
                }

                if (!formatted.Ok)
                {
                    ++result.Failed;
                    result.FirstError ??= $"{Path.GetFileName(path)}: " + formatted.Error;
                }
                else
                {
                    byte[] output = formatted.Output;
                    if (HasUtf8Bom(input) && !HasUtf8Bom(output))
                    {
                        output = PrependUtf8Bom(output);
                    }

                    ++result.Formatted;
                    if (!input.AsSpan().SequenceEqual(output))
                    {
                        File.WriteAllBytes(path, output);
                        ++result.Changed;
                    }
                }
            }
            catch (OperationCanceledException) when (token.IsCancellationRequested)
            {
                break;
            }
            catch (Exception ex)
                when (ex
                        is IOException
                            or UnauthorizedAccessException
                            or NotSupportedException
                            or ArgumentException
                            or PathTooLongException
                )
            {
                ++result.Failed;
                result.FirstError ??= $"{Path.GetFileName(path)}: {ex.Message}";
            }
            finally
            {
                Interlocked.Increment(ref operation.Completed);
            }
        }

        return result;
    }

    private bool PollWorkspaceFormat()
    {
        WorkspaceFormatOperation? operation = _workspaceFormatOperation;
        if (operation is null)
        {
            return false;
        }

        int total = Volatile.Read(ref operation.Total);
        int completed = Volatile.Read(ref operation.Completed);
        string currentFile = Volatile.Read(ref operation.CurrentFile);
        bool progressChanged =
            total != _workspaceFormatLastTotal
            || completed != _workspaceFormatLastCompleted
            || currentFile != _workspaceFormatLastFile;
        if (progressChanged)
        {
            _workspaceFormatLastTotal = total;
            _workspaceFormatLastCompleted = completed;
            _workspaceFormatLastFile = currentFile;
            if (total > 0)
            {
                int current = Math.Min(completed + 1, total);
                SetMessage(
                    currentFile.Length > 0
                        ? $"Formatting ({current}/{total}): {currentFile}"
                        : $"Formatting ({current}/{total})..."
                );
            }
        }

        if (!operation.Task.IsCompleted)
        {
            return progressChanged;
        }

        _workspaceFormatOperation = null;
        operation.Dispose();
        _workspaceFormatLastTotal = -1;
        _workspaceFormatLastCompleted = -1;
        _workspaceFormatLastFile = string.Empty;

        WorkspaceFormatResult result;
        try
        {
            result = operation.Task.GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
            SetMessage("Formatting cancelled.");
            return true;
        }
        catch (Exception ex)
        {
            SetError($"Format failed: {ex.Message}");
            return true;
        }

        if (result.Failed > 0)
        {
            string detail = result.FirstError.Length > 0 ? $": {result.FirstError}" : string.Empty;
            SetError(
                result.Formatted == 0
                    ? $"Formatting .{operation.Extension} files failed{detail}"
                    : $"Formatted {result.Formatted} .{operation.Extension} "
                        + $"{(result.Formatted == 1 ? "file" : "files")}, "
                        + $"{result.Failed} failed{detail}"
            );
            return true;
        }

        if (result.Formatted == 0)
        {
            if (result.Matched == 0)
            {
                SetMessage($"No .{operation.Extension} files found in the workspace.");
            }
            else if (result.SkippedOpen > 0 && result.SkippedOpen == total)
            {
                SetMessage("Skipped the open buffer.");
            }
            else
            {
                SetMessage($"No .{operation.Extension} files were formatted.");
            }
            return true;
        }

        string summary =
            $"Formatted {result.Formatted} .{operation.Extension} "
            + $"{(result.Formatted == 1 ? "file" : "files")}";
        if (result.Changed > 0)
        {
            summary += $", {result.Changed} changed";
        }
        if (result.Skipped > 0)
        {
            summary += $", {result.Skipped} skipped";
        }
        if (result.SkippedOpen > 0)
        {
            summary += ", open buffer skipped";
        }
        SetMessage(summary);
        return true;
    }

    private static bool TryNormalizeExtension(string argument, out string extension)
    {
        extension = argument.Trim();
        while (extension.StartsWith('.'))
        {
            extension = extension[1..];
        }

        if (extension.Length == 0)
        {
            return false;
        }

        foreach (char character in extension)
        {
            if (char.IsWhiteSpace(character) || character is '/' or '\\')
            {
                extension = string.Empty;
                return false;
            }
        }

        return true;
    }
}

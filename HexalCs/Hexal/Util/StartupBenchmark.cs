using System.Diagnostics;

namespace Hexal.Util;

public readonly record struct StartupBenchmarkEntry(
    string Category,
    string Name,
    double Milliseconds,
    string Detail
);

/// <summary>
/// Optional startup and lifecycle timings. The editor only creates this when
/// the configuration enables it, so disabled runs pay no measurement cost.
/// </summary>
public sealed class StartupBenchmark
{
    private readonly List<StartupBenchmarkEntry> _entries = new();

    public IReadOnlyList<StartupBenchmarkEntry> Entries => _entries;

    public void Add(string category, string name, double milliseconds, string detail = "")
    {
        _entries.Add(new StartupBenchmarkEntry(category, name, Math.Max(0, milliseconds), detail));
    }

    public void AddSince(string category, string name, long startTimestamp, string detail = "")
    {
        Add(category, name, Stopwatch.GetElapsedTime(startTimestamp).TotalMilliseconds, detail);
    }

    public IReadOnlyList<StartupBenchmarkEntry> SortedByDuration() =>
        _entries.OrderByDescending(entry => entry.Milliseconds).ToArray();
}

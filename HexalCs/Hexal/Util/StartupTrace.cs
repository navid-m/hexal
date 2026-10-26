using System.Diagnostics;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Text;

namespace Hexal.Util;

/// <summary>
/// Opt-in, high-detail startup tracing. It is enabled by <c>-vv</c> (or
/// <c>--verbose</c>) and writes every phase to <c>~/.hexal/hexal.log</c>, so a
/// cold-boot launch can be diagnosed after the fact. Disabled runs only pay a
/// single boolean check per call site.
/// </summary>
public static class StartupTrace
{
    private const string Prefix = "[startup] ";

    private static readonly object Sync = new();
    private static readonly long MainTimestamp = Stopwatch.GetTimestamp();
    private static bool _enabled;
    private static DateTime? _processStartUtc;
    private static double _firstLogMilliseconds = -1;

    public static bool Enabled => _enabled;

    /// <summary>
    /// Enables tracing when the command line contains <c>-vv</c>. Call this as
    /// early as possible (from <c>Main</c>) so the runtime's own startup cost is
    /// included in the process-age measurements.
    /// </summary>
    public static void Begin(string[] arguments)
    {
        if (!HasVerboseFlag(arguments))
        {
            return;
        }

        _enabled = true;
        _processStartUtc = TryGetProcessStartUtc();

        try
        {
            WriteHeader(arguments);
        }
        catch (Exception ex)
            when (ex
                is IOException
                    or UnauthorizedAccessException
                    or NotSupportedException
                    or ArgumentException
                    or InvalidOperationException
                    or System.Security.SecurityException
            )
        {
            // Tracing must never prevent the editor from starting.
        }
    }

    private static void WriteHeader(string[] arguments)
    {
        long firstLogStarted = Stopwatch.GetTimestamp();
        HexalPaths.Log($"{Prefix}=== verbose startup trace ===");
        _firstLogMilliseconds = Stopwatch.GetElapsedTime(firstLogStarted).TotalMilliseconds;

        Write($"first log write: {_firstLogMilliseconds:F1} ms to {HexalPaths.LogPath}");
        Write(
            $"process age at Main: {SinceProcessMilliseconds():F1} ms "
                + $"(exec -> Main; process start {FormatProcessStart()})"
        );
        Write($"version: {Program.Version} ({BuildInfo.CommitHash})");
        Write($"arguments: {FormatArguments(arguments)}");
        Write($"process path: {Environment.ProcessPath ?? "<unknown>"}");
        Write($"base directory: {AppContext.BaseDirectory}");
        Write($"working directory: {Environment.CurrentDirectory}");
        Write($"home directory: {HexalPaths.HomeDirectory}");
        Write(
            $"runtime: {RuntimeInformation.FrameworkDescription} "
                + $"({RuntimeInformation.ProcessArchitecture})"
        );
        Write($"os: {RuntimeInformation.OSDescription}");
        Write($"processor count: {Environment.ProcessorCount}");
        Write(
            $"bundle extract base: "
                + $"{Environment.GetEnvironmentVariable("DOTNET_BUNDLE_EXTRACT_BASE_DIR") ?? "<unset>"}"
        );
        Write($"temp path: {Path.GetTempPath()}");
        Write($"PATH: {Environment.GetEnvironmentVariable("PATH") ?? "<unset>"}");
    }

    public static long Timestamp() => _enabled ? Stopwatch.GetTimestamp() : 0;

    public static double SinceMainMilliseconds =>
        Stopwatch.GetElapsedTime(MainTimestamp).TotalMilliseconds;

    public static double SinceProcessMilliseconds()
    {
        if (_processStartUtc is not { } start)
        {
            return -1;
        }

        return Math.Max(0, (DateTime.UtcNow - start).TotalMilliseconds);
    }

    public static void Mark(string name, long startedTimestamp, string detail = "")
    {
        if (!_enabled || startedTimestamp == 0)
        {
            return;
        }

        double elapsed = Stopwatch.GetElapsedTime(startedTimestamp).TotalMilliseconds;
        string suffix = string.IsNullOrEmpty(detail) ? string.Empty : $" ({detail})";
        Write($"{name}: {elapsed:F1} ms{suffix}");
    }

    public static void Note(string message)
    {
        if (_enabled)
        {
            Write(message);
        }
    }

    public static void DumpBenchmark(StartupBenchmark? benchmark, string label)
    {
        if (!_enabled || benchmark is null)
        {
            return;
        }

        IReadOnlyList<StartupBenchmarkEntry> entries = benchmark.SortedByDuration();
        Write($"benchmark snapshot ({label}): {entries.Count} entries, slowest first");
        foreach (StartupBenchmarkEntry entry in entries)
        {
            string detail = string.IsNullOrEmpty(entry.Detail) ? string.Empty : $" ({entry.Detail})";
            Write(
                $"  bench [{entry.Category}] {entry.Name}: "
                    + $"{entry.Milliseconds:F1} ms{detail}"
            );
        }
    }

    private static void Write(string message)
    {
        string line = FormattableString.Invariant(
            $"{Prefix}[main+{SinceMainMilliseconds:F1}ms process+{SinceProcessMilliseconds():F1}ms] "
        );
        lock (Sync)
        {
            HexalPaths.Log(line + Sanitize(message));
        }
    }

    private static string Sanitize(string value) =>
        value.Replace('\r', ' ').Replace('\n', ' ');

    private static string FormatProcessStart() =>
        _processStartUtc is { } start
            ? start.ToString("yyyy-MM-ddTHH:mm:ss.fffZ", CultureInfo.InvariantCulture)
            : "<unavailable>";

    private static bool HasVerboseFlag(string[] arguments)
    {
        foreach (string argument in arguments)
        {
            if (argument == "--")
            {
                break;
            }

            if (argument is "-vv" or "--verbose")
            {
                return true;
            }
        }

        return false;
    }

    private static string FormatArguments(string[] arguments)
    {
        if (arguments.Length == 0)
        {
            return "<none>";
        }

        var builder = new StringBuilder();
        foreach (string argument in arguments)
        {
            if (builder.Length > 0)
            {
                builder.Append(' ');
            }

            if (
                argument.Length == 0
                || argument.Contains(' ')
                || argument.Contains('\t')
                || argument.Contains('"')
            )
            {
                builder.Append('"').Append(argument.Replace("\"", "\\\"", StringComparison.Ordinal)).Append('"');
            }
            else
            {
                builder.Append(argument);
            }
        }

        return builder.ToString();
    }

    private static DateTime? TryGetProcessStartUtc()
    {
        try
        {
            return Process.GetCurrentProcess().StartTime.ToUniversalTime();
        }
        catch (Exception ex)
            when (ex
                is InvalidOperationException
                    or NotSupportedException
                    or System.ComponentModel.Win32Exception
                    or UnauthorizedAccessException
            )
        {
            return null;
        }
    }
}

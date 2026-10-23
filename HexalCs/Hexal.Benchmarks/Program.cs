using System.Diagnostics;
using System.Globalization;
using System.Text;
using Hexal.Core;
using Hexal.Ui;
using Hexal.Util;

namespace Hexal.Benchmarks;

internal static class Program
{
    private static ulong s_sink;

    private struct Rng
    {
        public ulong State;

        public ulong Next()
        {
            State ^= State >> 12;
            State ^= State << 25;
            State ^= State >> 27;
            return State * 2685821657736338717UL;
        }
    }

    private sealed class Options
    {
        public string File = string.Empty;
        public int Lines = 100000;
        public int Reps = 3;
        public ulong Seed = 0x9E3779B97F4A7C15UL;
    }

    private static int Main(string[] args)
    {
        try
        {
            Options options = ParseOptions(args);
            Console.Out.WriteLine("IMPL\tcsharp");
            Console.Out.WriteLine($"LINES\t{options.Lines}");
            Console.Out.WriteLine($"FILE\t{options.File}");

            byte[] initial = MakeLines(options.Lines);
            RunPieceBenchmarks(initial, options);
            RunUnicodeBenchmark(options);
            RunDocumentBenchmarks(options);

            Console.Out.WriteLine($"SINK\t{s_sink}");
            return 0;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"csharp benchmark failed: {ex.Message}");
            return 1;
        }
    }

    private static Options ParseOptions(string[] args)
    {
        var options = new Options();
        for (int i = 0; i < args.Length; ++i)
        {
            switch (args[i])
            {
                case "--file" when i + 1 < args.Length:
                    options.File = args[++i];
                    break;
                case "--lines" when i + 1 < args.Length:
                    options.Lines = int.Parse(args[++i], CultureInfo.InvariantCulture);
                    break;
                case "--reps" when i + 1 < args.Length:
                    options.Reps = Math.Max(1, int.Parse(args[++i], CultureInfo.InvariantCulture));
                    break;
                case "--seed" when i + 1 < args.Length:
                    options.Seed = ulong.Parse(args[++i], CultureInfo.InvariantCulture);
                    break;
            }
        }
        return options;
    }

    private static byte[] MakeLines(int count)
    {
        var builder = new StringBuilder(count * 48);
        for (int i = 0; i < count; ++i)
        {
            builder.Append("This is test line ");
            builder.Append(i + 1);
            builder.Append(": Hexal performance testing.\n");
        }
        return Encoding.UTF8.GetBytes(builder.ToString());
    }

    private static void Measure(string name, long operations, int reps, Action setup, Action body)
    {
        setup();
        body(); // warm-up the JIT and caches

        var times = new List<double>(reps);
        for (int i = 0; i < reps; ++i)
        {
            setup();
            var watch = Stopwatch.StartNew();
            body();
            watch.Stop();
            times.Add(watch.Elapsed.TotalSeconds);
        }

        times.Sort();
        double median = times[times.Count / 2];
        Console.Out.WriteLine(
            $"METRIC\t{name}\t{median.ToString("R", CultureInfo.InvariantCulture)}\t{operations}"
        );
    }

    private static void RunPieceBenchmarks(byte[] initial, Options options)
    {
        var rng = new Rng { State = options.Seed };
        int insertOps = options.Lines >= 1_000_000 ? 100_000 : 200_000;
        int eraseOps = insertOps / 2;
        int lookupOps = options.Lines >= 1_000_000 ? 300_000 : 500_000;
        int extractOps = options.Lines >= 1_000_000 ? 100_000 : 200_000;

        var insertTable = new PieceTable();
        Action insertSetup = () =>
        {
            insertTable = new PieceTable();
            insertTable.SetOriginal(initial);
        };
        Action insertBody = () =>
        {
            for (int i = 0; i < insertOps; ++i)
            {
                long position = (long)(rng.Next() % (ulong)(insertTable.Size + 1));
                insertTable.Insert(position, "x"u8);
            }
            s_sink += (ulong)insertTable.Size;
        };
        Measure("piece_insert", insertOps, options.Reps, insertSetup, insertBody);

        var eraseTable = new PieceTable();
        Action eraseSetup = () =>
        {
            eraseTable = new PieceTable();
            eraseTable.SetOriginal(initial);
        };
        Action eraseBody = () =>
        {
            for (int i = 0; i < eraseOps && eraseTable.Size > 8; ++i)
            {
                long position = (long)(rng.Next() % (ulong)(eraseTable.Size - 1));
                long length = 1 + (long)(rng.Next() % 8);
                eraseTable.Erase(position, length);
            }
            s_sink += (ulong)eraseTable.Size;
        };
        Measure("piece_erase", eraseOps, options.Reps, eraseSetup, eraseBody);

        var lookupTable = new PieceTable();
        lookupTable.SetOriginal(initial);
        Action noSetup = () => { };
        Action lookupBody = () =>
        {
            for (int i = 0; i < lookupOps; ++i)
            {
                long offset = (long)(rng.Next() % (ulong)(lookupTable.Size + 1));
                long line = lookupTable.LineOfOffset(offset);
                long start = lookupTable.LineStart(line);
                s_sink += (ulong)(line + start);
            }
        };
        Measure("line_lookup", lookupOps, options.Reps, noSetup, lookupBody);

        long lineCount = lookupTable.NewlineCount;
        Action extractBody = () =>
        {
            for (int i = 0; i < extractOps; ++i)
            {
                long line = (long)(rng.Next() % (ulong)lineCount);
                long start = lookupTable.LineStart(line);
                long end = lookupTable.LineStart(line + 1);
                long length = end > start ? end - start - 1 : 0;
                byte[] text = lookupTable.Range(start, length);
                s_sink += (ulong)text.Length;
            }
        };
        Measure("line_extract", extractOps, options.Reps, noSetup, extractBody);
    }

    private static void RunUnicodeBenchmark(Options options)
    {
        byte[] text = Encoding.UTF8.GetBytes("ASCII αβγ 世界 😀 Привет 0123456789\n");
        const int iterations = 20000;
        long operations = (long)text.Length * iterations;

        void NoSetup() { }
        void Body()
        {
            for (int i = 0; i < iterations; ++i)
            {
                int offset = 0;
                while (offset < text.Length)
                {
                    UnicodeUtil.Decode(text, offset, out Rune rune, out int length);
                    offset += length;
                    s_sink += (ulong)rune.Value + (ulong)UnicodeUtil.DisplayWidth((uint)rune.Value);
                }
            }
        }

        Measure("unicode_decode", operations, options.Reps, NoSetup, Body);
    }

    private static void RunDocumentBenchmarks(Options options)
    {
        if (options.File.Length == 0)
        {
            return;
        }

        Document? opened = Document.Open(options.File, out string error);
        if (opened is null)
        {
            throw new InvalidOperationException("cannot open benchmark file: " + error);
        }

        void NoSetup() { }
        void OpenBody()
        {
            Document? document = Document.Open(options.File, out string openError);
            if (document is null)
            {
                throw new InvalidOperationException(openError);
            }
            s_sink += (ulong)document.Size;
        }
        Measure("doc_open", 1, options.Reps, NoSetup, OpenBody);

        string savePath = options.File + ".bench-save";
        int saveReps = options.Lines >= 1_000_000 ? 2 : 3;
        void SaveBody()
        {
            Document.SaveSnapshot? snapshot = opened.CreateSaveSnapshot(
                savePath,
                out string saveError
            );
            if (snapshot is null)
            {
                throw new InvalidOperationException(saveError);
            }
            if (!snapshot.WriteTo(out saveError))
            {
                throw new InvalidOperationException(saveError);
            }
        }
        Measure("doc_save", 1, saveReps, NoSetup, SaveBody);
        try
        {
            File.Delete(savePath);
        }
        catch (IOException)
        {
            // Best effort cleanup.
        }

        const int columns = 120;
        const int rows = 40;
        const int textRows = rows - 2;
        int frameCount = options.Lines >= 1_000_000 ? 600 : 1200;
        long lineCount = opened.LineCount;
        var screen = new Screen(columns, rows);
        var renderer = new Renderer();
        var textStyle = new Style();
        var statusStyle = new Style
        {
            Fg = 15,
            Bg = 4,
            Bold = true,
        };

        void RenderSetup()
        {
            screen.Resize(columns, rows);
            screen.Clear();
        }
        void RenderBody()
        {
            long maxFirst = lineCount > textRows ? lineCount - textRows : 0;
            for (long frame = 0; frame < frameCount; ++frame)
            {
                screen.Clear();
                long first = maxFirst == 0 ? 0 : (frame * 37) % maxFirst;

                for (int row = 0; row < textRows; ++row)
                {
                    long line = first + row;
                    if (line >= lineCount)
                    {
                        break;
                    }

                    screen.PutInt(0, row, line + 1, textStyle, 4);
                    byte[] text = opened.LineText(line);
                    screen.PutUtf8(5, row, text, textStyle, columns - 5);
                }

                screen.Fill(0, rows - 2, columns, 1, (uint)' ', statusStyle);
                screen.PutString(0, rows - 2, "NORMAL  benchmark  file.txt", statusStyle, columns);

                ReadOnlySpan<byte> output = renderer.Render(screen, frame == 0);
                s_sink += (ulong)output.Length;
            }
        }
        Measure("render_viewport", frameCount, options.Reps, RenderSetup, RenderBody);
    }
}

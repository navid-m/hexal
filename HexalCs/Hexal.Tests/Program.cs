using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json.Nodes;
using Hexal.Core;
using Hexal.Dap;
using Hexal.Languages;
using Hexal.Lsp;
using Hexal.Scripting;
using Hexal.Ui;
using Hexal.Util;

namespace Hexal.Tests;

internal static class Program
{
    private static int s_checks;
    private static int s_failures;

    private static int Main()
    {
        TestPieceTableBasics();
        TestPieceTableRandomized();
        TestPieceTableLargeInsertAcrossChunks();
        TestSnapshotIsolation();
        TestLargeLineIndex();
        TestUnicode();
        TestCommandParsing();
        TestDocumentLinesAndSaving();
        TestDocumentOpenNewFileAndBom();
        TestTrailingWhitespaceTrim();
        TestDocumentIndentSettings();
        TestUndoRedo();
        TestCompositeUndo();
        TestSaveFailureLeavesOriginal();
        TestFilesystemReadWrite();
        TestFileSearch();
        TestGitDiff();
        TestGitDiffCompute();
        TestUnifiedDiff();
        TestFilePreview();
        TestHexalConfig();
        TestFormatterRunner();
        TestLspPositions();
        TestLspDebugReport();
        TestLspStartsForNewFile();
        TestLspDocumentSymbols();
        TestLspCompletions();
        TestDapLldbBreakpoint();
        TestLanguageGrammars();
        TestThemes();
        TestPluginConfigStore();
        TestPluginMetadataAndToggle();
        TestPluginReload();
        TestPluginInstaller();
        TestPluginProcessAndPanel();
        TestLazyPluginLoading();
        TestGitLogPlugin();
        TestLazyCSharpPlugin();
        TestLuaLspRuntime();
        TestHexalPaths();

        Console.Out.WriteLine($"{s_checks - s_failures} of {s_checks} checks passed");
        if (s_failures != 0)
        {
            Console.Error.WriteLine($"{s_failures} check(s) failed");
            return 1;
        }
        return 0;
    }

    private static void Check(
        bool condition,
        string expression,
        [CallerFilePath] string file = "",
        [CallerLineNumber] int line = 0
    )
    {
        ++s_checks;
        if (!condition)
        {
            ++s_failures;
            Console.Error.WriteLine($"{file}:{line}: CHECK failed: {expression}");
        }
    }

    private static void CheckEqual<T>(
        T actual,
        T expected,
        string actualText,
        string expectedText,
        [CallerFilePath] string file = "",
        [CallerLineNumber] int line = 0
    )
    {
        ++s_checks;
        if (!EqualityComparer<T>.Default.Equals(actual, expected))
        {
            ++s_failures;
            Console.Error.WriteLine(
                $"{file}:{line}: CHECK_EQ failed: {actualText} == {expectedText}\n"
                    + $"  actual:   {actual}\n  expected: {expected}\n"
            );
        }
    }

    private static string Text(byte[] bytes) => Encoding.UTF8.GetString(bytes);

    private static void TestPieceTableBasics()
    {
        byte[] original = Encoding.UTF8.GetBytes("hello world\nsecond line\n");
        var table = new PieceTable();
        table.SetOriginal(original);

        CheckEqual(table.Size, (long)original.Length, "table.Size", "original.Length");
        CheckEqual(table.NewlineCount, 2L, "table.NewlineCount", "2");
        CheckEqual(table.LineStart(0), 0L, "table.LineStart(0)", "0");
        CheckEqual(table.LineStart(1), 12L, "table.LineStart(1)", "12");
        CheckEqual(table.LineStart(2), 24L, "table.LineStart(2)", "24");
        CheckEqual(table.LineStart(3), 24L, "table.LineStart(3)", "24");
        CheckEqual(table.LineOfOffset(0), 0L, "table.LineOfOffset(0)", "0");
        CheckEqual(table.LineOfOffset(11), 0L, "table.LineOfOffset(11)", "0");
        CheckEqual(table.LineOfOffset(12), 1L, "table.LineOfOffset(12)", "1");
        CheckEqual(table.LineOfOffset(24), 2L, "table.LineOfOffset(24)", "2");

        table.Insert(5, ","u8);
        CheckEqual(Text(table.ToArray()), "hello, world\nsecond line\n", "text", "comma insertion");

        table.Erase(0, 5);
        CheckEqual(Text(table.ToArray()), ", world\nsecond line\n", "text", "erase prefix");

        table.Insert(0, "well"u8);
        CheckEqual(Text(table.ToArray()), "well, world\nsecond line\n", "text", "insert prefix");

        CheckEqual(Text(table.Range(6, 5)), "world", "range", "world");
        CheckEqual(table.ByteAt(0), (byte)'w', "byteAt(0)", "w");
        CheckEqual(table.ByteAt(table.Size), (byte)0, "byteAt(size)", "0");

        table.Erase(0, table.Size);
        CheckEqual(table.Size, 0L, "size", "0");
        CheckEqual(table.NewlineCount, 0L, "newlines", "0");
        CheckEqual(table.LineStart(0), 0L, "lineStart(0)", "0");
    }

    private static void TestPieceTableRandomized()
    {
        var reference = new List<byte>();
        for (int i = 0; i < 80; ++i)
        {
            reference.AddRange(Encoding.UTF8.GetBytes($"line {i}\n"));
        }

        var table = new PieceTable();
        table.SetOriginal(reference.ToArray());

        string[] insertions =
        {
            "x",
            "YZ",
            "\n",
            "alpha\n",
            "\t",
            " ",
            "\r\n",
            "123",
            "longer text here",
        };
        var insertionBytes = insertions.Select(text => Encoding.UTF8.GetBytes(text)).ToArray();

        var random = new Random(0xC0FFEE);
        for (int step = 0; step < 3000; ++step)
        {
            bool doInsert = reference.Count == 0 || random.Next(3) != 0;
            if (doInsert)
            {
                int position = random.Next(reference.Count + 1);
                byte[] text = insertionBytes[random.Next(insertionBytes.Length)];
                reference.InsertRange(position, text);
                table.Insert(position, text);
            }
            else
            {
                int position = random.Next(reference.Count);
                int maxLength = Math.Min(8, reference.Count - position);
                int length = 1 + random.Next(maxLength);
                reference.RemoveRange(position, length);
                table.Erase(position, length);
            }

            if (step % 89 == 0)
            {
                Check(
                    table.ToArray().AsSpan().SequenceEqual(reference.ToArray()),
                    "randomized table == reference"
                );
            }
        }

        byte[] final = reference.ToArray();
        Check(table.ToArray().AsSpan().SequenceEqual(final), "final table == reference");
        CheckEqual(
            table.NewlineCount,
            (long)final.Count(b => b == '\n'),
            "newline count",
            "reference newline count"
        );

        long lineCount = table.NewlineCount + 1;
        for (long line = 0; line < lineCount; line += 17)
        {
            CheckEqual(
                table.LineStart(line),
                ExpectedLineStart(final, line),
                $"lineStart({line})",
                "expectedLineStart"
            );
        }
        CheckEqual(
            table.LineStart(lineCount),
            (long)final.Length,
            "lineStart(lineCount)",
            "reference length"
        );

        for (int i = 0; i < 200; ++i)
        {
            long offset = random.Next(final.Length + 1);
            CheckEqual(
                table.LineOfOffset(offset),
                ExpectedLineOfOffset(final, offset),
                $"lineOfOffset({offset})",
                "expectedLineOfOffset"
            );
        }
    }

    private static void TestPieceTableLargeInsertAcrossChunks()
    {
        var table = new PieceTable();
        table.SetOriginal("start\n"u8);

        var random = new Random(12345);
        var payload = new byte[2_500_000];
        for (int i = 0; i < payload.Length; ++i)
        {
            payload[i] = (byte)(random.Next(5) == 0 ? '\n' : 'a' + random.Next(26));
        }

        table.Insert(table.Size, payload);
        CheckEqual(table.Size, 6L + payload.Length, "size after large insert", "expected");
        Check(
            table.Range(6, payload.Length).AsSpan().SequenceEqual(payload),
            "large inserted range matches payload"
        );
        CheckEqual(
            table.NewlineCount,
            1L + payload.Count(b => b == '\n'),
            "large insert newline count",
            "expected"
        );

        long middleLine = table.LineOfOffset(1_000_000);
        long start = table.LineStart(middleLine);
        Check(
            start <= 1_000_000 && (start == 1_000_000 || table.ByteAt(start - 1) == '\n'),
            "offset maps to a valid line start"
        );
    }

    private static void TestSnapshotIsolation()
    {
        var table = new PieceTable();
        table.SetOriginal("hello\n"u8);
        table.Insert(0, "A"u8);

        PieceTable.Snapshot snapshot = table.CreateSnapshot();
        table.Insert(0, "B"u8);
        table.Insert(table.Size, "tail"u8);

        using var stream = new MemoryStream();
        snapshot.WriteTo(stream);
        CheckEqual(
            Encoding.UTF8.GetString(stream.ToArray()),
            "Ahello\n",
            "snapshot text",
            "Ahello"
        );
        CheckEqual(
            Text(table.ToArray()),
            "BAhello\ntail",
            "live text after snapshot",
            "BAhello\ntail"
        );

        // Appending to the add buffer after a snapshot must not change the
        // snapshot, even when the append crosses a storage chunk boundary.
        PieceTable.Snapshot beforeLargeAppend = table.CreateSnapshot();
        byte[] payload = new byte[1_500_000];
        Array.Fill(payload, (byte)'z');
        table.Insert(table.Size, payload);

        using var largeStream = new MemoryStream();
        beforeLargeAppend.WriteTo(largeStream);
        CheckEqual(largeStream.Length, 12L, "snapshot length after large append", "12");
        CheckEqual(
            Encoding.UTF8.GetString(largeStream.ToArray()),
            "BAhello\ntail",
            "snapshot unaffected by later append",
            "BAhello\ntail"
        );

        using var directory = new TempDir();
        string path = directory.Path("snapshot-doc.txt");
        Check(WriteBytes(path, "base\n"u8), "write snapshot document");
        Document? document = Document.Open(path, out string error);
        Check(document is not null, "open snapshot document");
        if (document is null)
        {
            return;
        }

        document.Insert(0, "x"u8);
        Document.SaveSnapshot? docSnapshot = document.CreateSaveSnapshot(
            directory.Path("saved-snapshot.txt"),
            out error
        );
        Check(docSnapshot is not null, "create document save snapshot");
        if (docSnapshot is null)
        {
            return;
        }

        document.Insert(0, "y"u8);
        Check(docSnapshot.WriteTo(out error), "write document snapshot");
        CheckEqual(
            Text(ReadBytes(directory.Path("saved-snapshot.txt"))),
            "xbase\n",
            "document snapshot contents",
            "xbase"
        );

        document.MarkSaved(
            directory.Path("saved-snapshot.txt"),
            docSnapshot.Pose,
            docSnapshot.Revision
        );
        Check(document.IsDirty, "edits after snapshot keep document dirty");
        CheckEqual(
            document.Path ?? "",
            directory.Path("saved-snapshot.txt"),
            "mark saved updates path",
            "snapshot path"
        );

        // Undo plus a new edit can return the undo stack to the same depth with
        // different contents. The revision check must keep the document dirty.
        using var branchDirectory = new TempDir();
        string branchPath = branchDirectory.Path("branch.txt");
        Check(WriteBytes(branchPath, "base\n"u8), "write branch snapshot document");
        Document? branch = Document.Open(branchPath, out error);
        Check(branch is not null, "open branch snapshot document");
        if (branch is null)
        {
            return;
        }

        branch.Insert(0, "x"u8);
        Document.SaveSnapshot? branchSnapshot = branch.CreateSaveSnapshot(
            branchDirectory.Path("branch-saved.txt"),
            out error
        );
        Check(branchSnapshot is not null, "create branch snapshot");
        if (branchSnapshot is null)
        {
            return;
        }

        Check(branch.Undo() is not null, "undo branch snapshot edit");
        branch.Insert(0, "y"u8);
        branch.MarkSaved(
            branchDirectory.Path("branch-saved.txt"),
            branchSnapshot.Pose,
            branchSnapshot.Revision
        );
        Check(branch.IsDirty, "same-depth redo branch stays dirty");
    }

    private static void TestLargeLineIndex()
    {
        var builder = new StringBuilder(600_000);
        for (int i = 0; i < 100_000; ++i)
        {
            builder.Append("line\n");
        }
        byte[] large = Encoding.UTF8.GetBytes(builder.ToString());

        var table = new PieceTable();
        table.SetOriginal(large);
        CheckEqual(table.NewlineCount, 100_000L, "large newline count", "100000");
        CheckEqual(table.LineStart(50_000), 250_000L, "lineStart(50000)", "250000");
        CheckEqual(table.LineOfOffset(250_000), 50_000L, "lineOfOffset(250000)", "50000");
        CheckEqual(table.LineStart(99_999), 499_995L, "lineStart(99999)", "499995");
        CheckEqual(table.LineStart(100_000), (long)large.Length, "lineStart(100000)", "size");

        table.Insert(250_000, "prefix\n"u8);
        CheckEqual(table.LineStart(50_000), 250_000L, "lineStart(50000) after insert", "250000");
        CheckEqual(table.LineStart(50_001), 250_007L, "lineStart(50001) after insert", "250007");
        CheckEqual(table.LineOfOffset(250_007), 50_001L, "lineOfOffset(250007)", "50001");
    }

    private static void TestUnicode()
    {
        byte[] eAcute = { 0xC3, 0xA9 };
        Check(UnicodeUtil.Decode(eAcute, 0, out Rune rune, out int length), "decode e-acute");
        CheckEqual((uint)rune.Value, 0xE9u, "rune", "0xE9");
        CheckEqual(length, 2, "length", "2");

        byte[] invalid = { 0x80 };
        Check(!UnicodeUtil.Decode(invalid, 0, out rune, out length), "decode invalid");
        CheckEqual((uint)rune.Value, 0xFFFDu, "invalid rune", "U+FFFD");
        CheckEqual(length, 1, "invalid length", "1");

        byte[] wide = { 0xE7, 0x95, 0x8C };
        Check(UnicodeUtil.Decode(wide, 0, out rune, out length), "decode wide");
        CheckEqual((uint)rune.Value, 0x754Cu, "wide rune", "0x754C");
        CheckEqual(UnicodeUtil.DisplayWidth(0x754C), 2, "wide display width", "2");
        CheckEqual(UnicodeUtil.DisplayWidth('a'), 1, "ascii display width", "1");

        CheckEqual(UnicodeUtil.PreviousBoundary(eAcute, 2), 0, "previous boundary", "0");
        CheckEqual(UnicodeUtil.NextBoundary(eAcute, 0), 2, "next boundary", "2");

        byte[] line = Encoding.UTF8.GetBytes("a\t界");
        CheckEqual(UnicodeUtil.VisualColumn(line, line.Length, 4), 6L, "visual column", "6");
        CheckEqual(
            UnicodeUtil.ByteOffsetForVisualColumn(line, 4, 4),
            2,
            "byte offset for column 4",
            "2"
        );
        CheckEqual(
            UnicodeUtil.ByteOffsetForVisualColumn(line, 5, 4),
            2,
            "byte offset for column 5",
            "2"
        );
        CheckEqual(
            UnicodeUtil.ByteOffsetForVisualColumn(line, 6, 4),
            line.Length,
            "byte offset for column 6",
            "length"
        );

        Span<byte> encoded = stackalloc byte[4];
        CheckEqual(UnicodeUtil.Encode('A', encoded), 1, "encode A", "1");
        CheckEqual(UnicodeUtil.Encode(0x754C, encoded), 3, "encode wide", "3");
    }

    private static void TestCommandParsing()
    {
        ExParseResult result = ExCommandParser.Parse("e \"src/my file.cpp\"");
        Check(result.Ok, "parse quoted path ok");
        CheckEqual(result.Command.Name, "e", "command name", "e");
        CheckEqual(result.Command.Arguments.Count, 1, "argument count", "1");
        CheckEqual(result.Command.Arguments[0], "src/my file.cpp", "argument", "path");

        result = ExCommandParser.Parse("w 'a b.txt'");
        Check(result.Ok, "parse single quoted ok");
        CheckEqual(result.Command.Arguments[0], "a b.txt", "single quoted argument", "a b.txt");

        result = ExCommandParser.Parse("w C:\\tmp\\file.txt");
        Check(result.Ok, "parse Windows path ok");
        CheckEqual(result.Command.Arguments[0], "C:\\tmp\\file.txt", "Windows path", "path");

        result = ExCommandParser.Parse("w \"say \\\"hi\\\"\"");
        Check(result.Ok, "parse escaped quote ok");
        CheckEqual(result.Command.Arguments[0], "say \"hi\"", "escaped quote", "say \"hi\"");

        result = ExCommandParser.Parse("e \"\\\\server\\share\"");
        Check(result.Ok, "parse UNC path ok");
        CheckEqual(result.Command.Arguments[0], "\\\\server\\share", "UNC path", "path");

        result = ExCommandParser.Parse("e \"unterminated");
        Check(!result.Ok, "unterminated quote fails");
        Check(result.Error.Length > 0, "unterminated quote error");

        result = ExCommandParser.Parse("   ");
        Check(result.Ok, "blank command ok");
        CheckEqual(result.Command.Name, "", "blank command name", "empty");

        result = ExCommandParser.Parse("q!");
        Check(result.Ok, "q! parses");
        CheckEqual(result.Command.Name, "q!", "q! name", "q!");
    }

    private static void TestDocumentLinesAndSaving()
    {
        using var directory = new TempDir();

        string crlfPath = directory.Path("crlf.txt");
        Check(WriteBytes(crlfPath, "alpha\r\nbeta\r\ngamma"u8), "write CRLF file");

        Document? document = Document.Open(crlfPath, out string error);
        Check(document is not null, "open CRLF document");
        if (document is null)
        {
            Console.Error.WriteLine(error);
            return;
        }

        CheckEqual(document.Eol, "\r\n", "eol", "CRLF");
        CheckEqual(document.LineCount, 3L, "line count", "3");
        CheckEqual(document.LineStart(0), 0L, "lineStart(0)", "0");
        CheckEqual(document.LineStart(1), 7L, "lineStart(1)", "7");
        CheckEqual(document.LineStart(2), 13L, "lineStart(2)", "13");
        CheckEqual(document.LineEnd(0), 5L, "lineEnd(0)", "5");
        CheckEqual(document.LineEnd(1), 11L, "lineEnd(1)", "11");
        CheckEqual(document.LineEnd(2), 18L, "lineEnd(2)", "18");
        CheckEqual(Text(document.LineText(0)), "alpha", "line 0", "alpha");
        CheckEqual(Text(document.LineText(1)), "beta", "line 1", "beta");
        CheckEqual(Text(document.LineText(2)), "gamma", "line 2", "gamma");
        CheckEqual(document.LineOfOffset(6), 0L, "lineOfOffset(6)", "0");
        CheckEqual(document.LineOfOffset(7), 1L, "lineOfOffset(7)", "1");
        CheckEqual(document.LineOfOffset(18), 2L, "lineOfOffset(18)", "2");
        Check(!document.IsDirty, "new document is clean");

        document.Insert(document.LineEnd(0), "\r\n"u8);
        CheckEqual(document.LineCount, 4L, "line count after newline", "4");
        CheckEqual(Text(document.LineText(1)), "", "new empty line", "empty");
        CheckEqual(Text(document.LineText(2)), "beta", "shifted line", "beta");

        Check(document.Save(out error), "save CRLF document");
        CheckEqual(
            Text(ReadBytes(crlfPath)),
            "alpha\r\n\r\nbeta\r\ngamma",
            "saved CRLF",
            "expected"
        );
        Check(!document.IsDirty, "document is clean after save");

        string lfPath = directory.Path("lf.txt");
        Check(WriteBytes(lfPath, "x\ny"u8), "write LF file");
        Document? lf = Document.Open(lfPath, out error);
        Check(lf is not null, "open LF document");
        if (lf is not null)
        {
            CheckEqual(lf.LineCount, 2L, "LF line count", "2");
            CheckEqual(lf.LineStart(1), 2L, "LF lineStart(1)", "2");
            CheckEqual(lf.LineEnd(0), 1L, "LF lineEnd(0)", "1");
            CheckEqual(lf.LineEnd(1), 3L, "LF lineEnd(1)", "3");
        }

        string trailingPath = directory.Path("trailing.txt");
        Check(WriteBytes(trailingPath, "x\n"u8), "write trailing newline file");
        Document? trailing = Document.Open(trailingPath, out error);
        Check(trailing is not null, "open trailing document");
        if (trailing is not null)
        {
            CheckEqual(trailing.LineCount, 2L, "trailing line count", "2");
            CheckEqual(trailing.LineStart(1), 2L, "trailing lineStart(1)", "2");
            CheckEqual(trailing.LineEnd(0), 1L, "trailing lineEnd(0)", "1");
            CheckEqual(trailing.LineEnd(1), 2L, "trailing lineEnd(1)", "2");
        }
    }

    private static void TestDocumentOpenNewFileAndBom()
    {
        using var directory = new TempDir();

        string newPath = directory.Path("new.txt");
        Document? created = Document.Open(newPath, out string error);
        Check(created is not null, "create new document");
        if (created is null)
        {
            Console.Error.WriteLine(error);
            return;
        }
        Check(created.HasFile, "new document has file");
        CheckEqual(created.Size, 0L, "new size", "0");
        CheckEqual(created.LineCount, 1L, "new line count", "1");
        Check(!created.IsDirty, "new document clean");

        created.Insert(0, "hello\n"u8);
        Check(created.IsDirty, "new document dirty after edit");
        Check(created.Save(out error), "save new document");
        Check(!created.IsDirty, "new document clean after save");
        CheckEqual(Text(ReadBytes(newPath)), "hello\n", "new file contents", "hello");

        string missingParent = System.IO.Path.Combine(directory.Path("missing"), "file.txt");
        Document? missing = Document.Open(missingParent, out error);
        Check(missing is null, "missing parent rejected");
        Check(error.Contains("parent directory"), "missing parent error");

        string bomPath = directory.Path("bom.txt");
        Check(
            WriteBytes(bomPath, new byte[] { 0xEF, 0xBB, 0xBF, (byte)'h', (byte)'i', (byte)'\n' }),
            "write BOM file"
        );
        Document? bom = Document.Open(bomPath, out error);
        Check(bom is not null, "open BOM document");
        if (bom is not null)
        {
            Check(bom.HasBom, "BOM detected");
            CheckEqual(Text(bom.Text()), "hi\n", "BOM stripped in memory", "hi");
            Check(bom.Save(out error), "save BOM document");
            Check(
                ReadBytes(bomPath)
                    .AsSpan()
                    .SequenceEqual(
                        new byte[] { 0xEF, 0xBB, 0xBF, (byte)'h', (byte)'i', (byte)'\n' }
                    ),
                "BOM preserved on save"
            );
        }
    }

    private static void TestTrailingWhitespaceTrim()
    {
        using var directory = new TempDir();
        string path = directory.Path("trim.txt");
        Check(
            WriteBytes(path, "alpha  \n\tbeta\t \n   \nkeep\nno newline  "u8),
            "write trailing whitespace file"
        );

        Document? document = Document.Open(path, out string error);
        Check(document is not null, $"open trailing whitespace document ({error})");
        if (document is null)
        {
            return;
        }

        CheckEqual(document.TrimTrailingWhitespace(), 4, "trimmed line count", "4");
        CheckEqual(
            Text(document.Text()),
            "alpha\n\tbeta\n\nkeep\nno newline",
            "trimmed document",
            "expected"
        );
        Check(document.IsDirty, "trim marks document dirty");
        CheckEqual(document.TrimTrailingWhitespace(), 0, "second trim changes nothing", "0");
    }

    private static void TestDocumentIndentSettings()
    {
        var document = new Document();

        Check(!document.UseTabs, "documents default to spaces");
        CheckEqual(document.IndentWidth, 4, "default indent width", "4");
        CheckEqual(document.TabWidth, 4, "default tab width", "4");
        CheckEqual(Text(document.IndentUnit()), "    ", "default indent unit", "four spaces");

        document.SetIndentTabs(8);
        Check(document.UseTabs, "tabs registered");
        CheckEqual(document.TabWidth, 8, "tab width set", "8");
        CheckEqual(document.IndentWidth, 8, "tab mode indent width", "8");
        CheckEqual(Text(document.IndentUnit()), "\t", "tab indent unit", "one tab");
        CheckEqual(document.UnindentLength("\t    x"u8), 1, "tab-indented line loses one tab", "1");
        CheckEqual(document.UnindentLength("  x"u8), 2, "tab mode still unindents spaces", "2");

        document.SetIndentSpaces(4);
        Check(!document.UseTabs, "spaces registered");
        CheckEqual(document.TabWidth, 8, "space mode keeps visual tab width", "8");
        CheckEqual(document.IndentWidth, 4, "space indent width set", "4");
        CheckEqual(Text(document.IndentUnit()), "    ", "space indent unit", "four spaces");
        CheckEqual(
            document.UnindentLength("      x"u8),
            4,
            "space indentation trims one level",
            "4"
        );
        CheckEqual(document.UnindentLength("\tx"u8), 1, "a leading tab counts as one level", "1");

        document.SetIndentSpaces(0);
        CheckEqual(document.IndentWidth, 1, "indent width is clamped", "1");
    }

    private static void TestUndoRedo()
    {
        using var directory = new TempDir();
        string path = directory.Path("undo.txt");
        Check(WriteBytes(path, "abc"u8), "write undo file");

        Document? loaded = Document.Open(path, out string error);
        Check(loaded is not null, "open undo file");
        if (loaded is null)
        {
            Console.Error.WriteLine(error);
            return;
        }

        Document document = loaded;
        document.Insert(3, "d"u8);
        document.Insert(4, "e"u8);
        document.Insert(5, "f"u8);
        CheckEqual(Text(document.Text()), "abcdef", "typed text", "abcdef");
        Check(document.IsDirty, "dirty after typing");

        long? hint = document.Undo();
        Check(hint is not null, "undo available");
        CheckEqual(hint ?? -1, 3L, "undo hint", "3");
        CheckEqual(Text(document.Text()), "abc", "undo text", "abc");
        Check(!document.IsDirty, "clean after undo to saved state");

        hint = document.Redo();
        Check(hint is not null, "redo available");
        CheckEqual(hint ?? -1, 6L, "redo hint", "6");
        CheckEqual(Text(document.Text()), "abcdef", "redo text", "abcdef");

        document.Erase(5, 1);
        document.Erase(4, 1);
        CheckEqual(Text(document.Text()), "abcd", "erase text", "abcd");
        Check(document.Undo() is not null, "undo erase group");
        CheckEqual(Text(document.Text()), "abcdef", "undo erase", "abcdef");
        Check(document.Redo() is not null, "redo erase group");
        CheckEqual(Text(document.Text()), "abcd", "redo erase", "abcd");

        Check(document.Save(out error), "save undo document");
        Check(!document.IsDirty, "clean after save");
        Check(document.Undo() is not null, "undo after save");
        CheckEqual(Text(document.Text()), "abcdef", "undo after save text", "abcdef");
        Check(document.IsDirty, "dirty after undo past save");
        Check(document.Redo() is not null, "redo after save");
        Check(!document.IsDirty, "clean after redo to save point");
        CheckEqual(Text(document.Text()), "abcd", "redo after save text", "abcd");

        // A new edit after undo must discard the redo branch.
        using var secondDirectory = new TempDir();
        string secondPath = secondDirectory.Path("undo2.txt");
        Check(WriteBytes(secondPath, "abc"u8), "write branch file");
        Document? second = Document.Open(secondPath, out error);
        Check(second is not null, "open branch file");
        if (second is null)
        {
            return;
        }

        Document branch = second;
        branch.Insert(3, "def"u8);
        CheckEqual(Text(branch.Text()), "abcdef", "branch text", "abcdef");
        Check(branch.Undo() is not null, "branch undo");
        CheckEqual(Text(branch.Text()), "abc", "branch undo text", "abc");
        Check(branch.CanRedo, "branch can redo");

        branch.Insert(branch.Size, "Z"u8);
        CheckEqual(Text(branch.Text()), "abcZ", "branch new edit", "abcZ");
        Check(!branch.CanRedo, "redo branch discarded");

        branch.BreakUndoGroup();
        branch.Replace(0, 3, "XYZ"u8);
        CheckEqual(Text(branch.Text()), "XYZZ", "replace text", "XYZZ");
        Check(branch.Undo() is not null, "undo replace");
        CheckEqual(Text(branch.Text()), "abcZ", "undo replace text", "abcZ");
        Check(branch.Redo() is not null, "redo replace");
        CheckEqual(Text(branch.Text()), "XYZZ", "redo replace text", "XYZZ");
    }

    private static void TestCompositeUndo()
    {
        using var directory = new TempDir();
        string path = directory.Path("composite.txt");
        Check(WriteBytes(path, "0123456789"u8), "write composite file");

        Document? loaded = Document.Open(path, out string error);
        Check(loaded is not null, "open composite file");
        if (loaded is null)
        {
            Console.Error.WriteLine(error);
            return;
        }

        Document document = loaded;

        // Composite edits are applied from the highest offset to the lowest by
        // multi-cursor operations. They must undo and redo in application order.
        document.BeginCompositeEdit();
        document.Insert(5, "X"u8);
        document.Insert(0, "Y"u8);
        document.EndCompositeEdit();
        CheckEqual(Text(document.Text()), "Y01234X56789", "composite text", "Y01234X56789");

        Check(document.Undo() is not null, "undo composite");
        CheckEqual(Text(document.Text()), "0123456789", "undo composite text", "0123456789");
        Check(document.Redo() is not null, "redo composite");
        CheckEqual(Text(document.Text()), "Y01234X56789", "redo composite text", "Y01234X56789");

        // Nested composite edits still form a single undo step.
        document.BeginCompositeEdit();
        document.BeginCompositeEdit();
        document.Erase(0, 2);
        document.EndCompositeEdit();
        document.EndCompositeEdit();
        CheckEqual(Text(document.Text()), "1234X56789", "nested composite text", "1234X56789");
        Check(document.Undo() is not null, "undo nested composite");
        CheckEqual(
            Text(document.Text()),
            "Y01234X56789",
            "undo nested composite text",
            "Y01234X56789"
        );
    }

    private static void TestSaveFailureLeavesOriginal()
    {
        using var directory = new TempDir();
        string path = directory.Path("safe.txt");
        Check(WriteBytes(path, "original\n"u8), "write safe file");

        Document? loaded = Document.Open(path, out string error);
        Check(loaded is not null, "open safe file");
        if (loaded is null)
        {
            return;
        }

        Document document = loaded;
        document.Insert(0, "changed "u8);
        string missing = System.IO.Path.Combine(directory.Path("missing"), "out.txt");
        Check(!document.SaveAs(missing, out error), "save to missing directory fails");
        Check(error.Length > 0, "save failure has error");
        CheckEqual(Text(ReadBytes(path)), "original\n", "original untouched", "original");
        CheckEqual(
            Text(document.Text()),
            "changed original\n",
            "in-memory text retained",
            "changed"
        );
        Check(document.IsDirty, "still dirty after failed save");

        Check(document.Save(out error), "save after failure");
        CheckEqual(Text(ReadBytes(path)), "changed original\n", "saved contents", "changed");
        Check(!document.IsDirty, "clean after successful save");
    }

    private static void TestFilesystemReadWrite()
    {
        using var directory = new TempDir();
        string path = directory.Path("atomic.txt");
        Check(WriteBytes(path, "old"u8), "write atomic file");

        Check(
            FileSystemUtil.AtomicWrite(path, stream => stream.Write("new"u8), out string error),
            "atomic write"
        );
        CheckEqual(Text(ReadBytes(path)), "new", "atomic contents", "new");

        string missing = System.IO.Path.Combine(directory.Path("nope"), "x.txt");
        Check(
            !FileSystemUtil.AtomicWrite(missing, stream => stream.Write("x"u8), out error),
            "atomic write to missing directory fails"
        );

        Check(FileSystemUtil.ReadFile(path, out byte[] contents, out error), "read file");
        CheckEqual(Text(contents), "new", "read contents", "new");
    }

    private static void TestFileSearch()
    {
        using var directory = new TempDir();
        string source = directory.Path("src");
        string ignored = directory.Path("build");
        string generated = directory.Path("generated");
        Directory.CreateDirectory(source);
        Directory.CreateDirectory(ignored);
        Directory.CreateDirectory(generated);

        Check(
            WriteBytes(directory.Path("root.txt"), "first line\nsecond line\n"u8),
            "write search root file"
        );
        Check(
            WriteBytes(System.IO.Path.Combine(source, "code.cs"), "alpha\n  beta needle gamma\n"u8),
            "write search nested file"
        );
        Check(
            WriteBytes(
                System.IO.Path.Combine(ignored, "skip.cs"),
                "needle in ignored directory\n"u8
            ),
            "write ignored file"
        );
        Check(
            WriteBytes(
                directory.Path(".gitignore"),
                "ignored.txt\n*.log\n!keep.log\ngenerated/\n"u8
            ),
            "write gitignore file"
        );
        Check(
            WriteBytes(directory.Path("ignored.txt"), "needle in ignored file\n"u8),
            "write gitignored file"
        );
        Check(
            WriteBytes(directory.Path("drop.log"), "needle in ignored log\n"u8),
            "write ignored log file"
        );
        Check(WriteBytes(directory.Path("keep.log"), "kept log\n"u8), "write negated log file");
        Check(
            WriteBytes(
                System.IO.Path.Combine(generated, "skip.cs"),
                "needle in generated directory\n"u8
            ),
            "write generated file"
        );

        List<string> files = FileSearch.EnumerateFiles(
            directory.Path("."),
            FileSearch.DefaultMaxFiles,
            CancellationToken.None
        );

        Check(
            files.Contains(System.IO.Path.Combine("src", "code.cs")),
            "nested file is enumerated"
        );
        Check(files.Contains("root.txt"), "root file is enumerated");
        Check(
            !files.Any(path => path.StartsWith("build", StringComparison.Ordinal)),
            "ignored directory is skipped"
        );
        Check(!files.Contains("ignored.txt"), "gitignored file is skipped");
        Check(!files.Contains("drop.log"), "gitignore wildcard is applied");
        Check(files.Contains("keep.log"), "gitignore negation is applied");
        Check(
            !files.Any(path => path.StartsWith("generated", StringComparison.Ordinal)),
            "gitignored directory is skipped"
        );

        List<string> allFiles = FileSearch.EnumerateFiles(
            directory.Path("."),
            FileSearch.DefaultMaxFiles,
            showIgnored: true,
            CancellationToken.None
        );
        Check(allFiles.Contains("ignored.txt"), "showIgnored includes gitignored file");
        Check(
            allFiles.Any(path => path.StartsWith("generated", StringComparison.Ordinal)),
            "showIgnored includes gitignored directory"
        );
        Check(
            allFiles.Any(path => path.StartsWith("build", StringComparison.Ordinal)),
            "showIgnored includes built-in ignored directory"
        );

        List<FileSearchMatch> matches = FileSearch.Search(
            directory.Path("."),
            files,
            "needle",
            FileSearch.DefaultMaxResults,
            CancellationToken.None
        );

        CheckEqual(matches.Count, 1, "matches.Count", "1");
        CheckEqual(
            matches[0].Path,
            System.IO.Path.Combine("src", "code.cs"),
            "match.Path",
            "nested code.cs"
        );
        CheckEqual(matches[0].Line, 1L, "match.Line", "1");
        CheckEqual(matches[0].Column, 7L, "match.Column", "7");
        Check(
            matches[0].Preview.Contains("beta needle", StringComparison.Ordinal),
            "match preview"
        );

        List<FileSearchMatch> caseSensitiveMiss = FileSearch.Search(
            directory.Path("."),
            files,
            "NEEDLE",
            FileSearch.DefaultMaxResults,
            CancellationToken.None,
            matchCase: true
        );
        CheckEqual(caseSensitiveMiss.Count, 0, "case-sensitive misses", "0");

        List<FileSearchMatch> caseSensitiveHit = FileSearch.Search(
            directory.Path("."),
            files,
            "needle",
            FileSearch.DefaultMaxResults,
            CancellationToken.None,
            matchCase: true
        );
        CheckEqual(caseSensitiveHit.Count, 1, "case-sensitive hit", "1");
    }

    private static void TestGitDiff()
    {
        using var directory = new TempDir();
        string repo = directory.Path("repo");
        Directory.CreateDirectory(repo);
        if (!RunGit(repo, "init", "-q"))
        {
            return;
        }

        RunGit(repo, "config", "user.email", "test@example.com");
        RunGit(repo, "config", "user.name", "Hexal Test");

        string changed = System.IO.Path.Combine(repo, "changed.txt");
        Check(WriteBytes(changed, "one\ntwo\nthree\n"u8), "write git diff file");
        Check(RunGit(repo, "add", "changed.txt"), "stage git diff file");
        Check(RunGit(repo, "commit", "-q", "-m", "initial"), "commit git diff file");

        Check(WriteBytes(changed, "one\nTWO\nthree\nfour\n"u8), "modify git diff file");
        Dictionary<long, GitDiffLineKind> diff = GitDiff.Load(changed, 4);
        Check(
            diff.TryGetValue(1, out GitDiffLineKind kind) && kind == GitDiffLineKind.Modified,
            "changed line is marked modified"
        );
        Check(
            diff.TryGetValue(3, out kind) && kind == GitDiffLineKind.Added,
            "appended line is marked added"
        );

        string deleted = System.IO.Path.Combine(repo, "deleted.txt");
        Check(WriteBytes(deleted, "a\nb\nc\n"u8), "write deletion file");
        Check(RunGit(repo, "add", "deleted.txt"), "stage deletion file");
        Check(RunGit(repo, "commit", "-q", "-m", "deletion baseline"), "commit deletion file");

        Check(WriteBytes(deleted, "a\nc\n"u8), "delete middle line");
        diff = GitDiff.Load(deleted, 2);
        Check(
            diff.TryGetValue(1, out kind) && kind == GitDiffLineKind.Deleted,
            "deleted line is marked"
        );
    }

    private static void TestGitDiffCompute()
    {
        ulong[] oldLines = LineHashes("a", "b", "c");
        ulong[] newLines = LineHashes("a", "B", "c", "d");
        Dictionary<long, GitDiffLineKind> diff = GitDiff.Compute(oldLines, newLines);
        Check(
            diff.TryGetValue(1, out GitDiffLineKind kind) && kind == GitDiffLineKind.Modified,
            "live diff marks a changed line"
        );
        Check(
            diff.TryGetValue(3, out kind) && kind == GitDiffLineKind.Added,
            "live diff marks an appended line"
        );

        diff = GitDiff.Compute(LineHashes("a", "b", "c"), LineHashes("a", "c"));
        Check(
            diff.TryGetValue(1, out kind) && kind == GitDiffLineKind.Deleted,
            "live diff marks a deleted line"
        );

        diff = GitDiff.Compute(LineHashes("a"), LineHashes("a", "b", "c"));
        Check(
            diff.TryGetValue(1, out kind)
                && kind == GitDiffLineKind.Added
                && diff.TryGetValue(2, out kind)
                && kind == GitDiffLineKind.Added,
            "live diff marks inserted lines"
        );
    }

    private static void TestUnifiedDiff()
    {
        string modified = UnifiedDiff.Create("old.txt", "a\nb\nc\n", "new.txt",
            "a\nB\nc\n");
        Check(modified.Contains("--- old.txt", StringComparison.Ordinal),
            "unified diff old header");
        Check(modified.Contains("+++ new.txt", StringComparison.Ordinal),
            "unified diff new header");
        Check(modified.Contains("@@ -1,3 +1,3 @@", StringComparison.Ordinal),
            "unified diff hunk header");
        Check(modified.Contains("\n-b\n", StringComparison.Ordinal) &&
              modified.Contains("\n+B\n", StringComparison.Ordinal),
            "unified diff changed lines");

        string inserted = UnifiedDiff.Create("old.txt", "a\nc\n", "new.txt",
            "a\nb\nc\n");
        Check(inserted.Contains("@@ -1,2 +1,3 @@", StringComparison.Ordinal) &&
              inserted.Contains("\n+b\n", StringComparison.Ordinal),
            "unified diff inserted line");

        string equal = UnifiedDiff.Create("old.txt", "a\n", "new.txt", "a\n");
        Check(!equal.Contains("@@", StringComparison.Ordinal),
            "unified diff omits identical hunk");
    }

    private static ulong[] LineHashes(params string[] lines)
    {
        var hashes = new ulong[lines.Length];
        for (int index = 0; index < lines.Length; ++index)
        {
            hashes[index] = GitDiff.HashLine(Encoding.UTF8.GetBytes(lines[index]));
        }
        return hashes;
    }

    private static bool RunGit(string directory, params string[] arguments)
    {
        try
        {
            var startInfo = new System.Diagnostics.ProcessStartInfo
            {
                FileName = "git",
                WorkingDirectory = directory,
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            };

            foreach (string argument in arguments)
            {
                startInfo.ArgumentList.Add(argument);
            }

            using System.Diagnostics.Process? process = System.Diagnostics.Process.Start(startInfo);
            if (process is null)
            {
                return false;
            }

            process.StandardInput.Close();
            _ = process.StandardOutput.ReadToEnd();
            _ = process.StandardError.ReadToEnd();
            return process.WaitForExit(10_000) && process.ExitCode == 0;
        }
        catch (Exception ex)
            when (ex
                    is System.ComponentModel.Win32Exception
                        or IOException
                        or InvalidOperationException
                        or NotSupportedException
            )
        {
            return false;
        }
    }

    private static void TestFilePreview()
    {
        using var directory = new TempDir();
        string path = directory.Path("preview.txt");
        var content = new StringBuilder();
        for (int i = 0; i < 25; ++i)
        {
            content.Append($"line {i}\n");
        }
        Check(WriteBytes(path, Encoding.UTF8.GetBytes(content.ToString())), "write preview file");

        FilePreview preview = FileSearch.ReadPreview(path);
        CheckEqual(preview.Error, string.Empty, "preview.Error", "empty");
        CheckEqual(
            preview.Lines.Count,
            FileSearch.DefaultPreviewLines,
            "preview.Lines.Count",
            "10"
        );
        CheckEqual(preview.Lines[0], "line 0", "preview first line", "line 0");
        CheckEqual(preview.Lines[9], "line 9", "preview tenth line", "line 9");
        Check(preview.Truncated, "preview is truncated");

        string shortPath = directory.Path("short.txt");
        Check(WriteBytes(shortPath, "one\ntwo\n"u8), "write short preview file");
        FilePreview shortPreview = FileSearch.ReadPreview(shortPath);
        CheckEqual(shortPreview.Lines.Count, 2, "short preview lines", "2");
        Check(!shortPreview.Truncated, "short preview is not truncated");

        string binaryPath = directory.Path("binary.bin");
        Check(
            WriteBytes(binaryPath, new byte[] { (byte)'a', 0, (byte)'b' }),
            "write binary preview file"
        );
        FilePreview binaryPreview = FileSearch.ReadPreview(binaryPath);
        Check(binaryPreview.Binary, "binary preview detected");

        FilePreview missing = FileSearch.ReadPreview(directory.Path("missing.txt"));
        Check(missing.Error.Length > 0, "missing preview reports an error");
    }

    private static void TestHexalConfig()
    {
        Check(
            HexalConfig.TryParse(
                "{\"formatOnSave\": true}",
                "test.hexcfg",
                out HexalConfig? config,
                out string error
            ),
            "parse config"
        );
        CheckEqual(error, string.Empty, "config error", "empty");
        Check(config is not null && config.FormatOnSave, "config formatOnSave root");

        Check(
            HexalConfig.TryParse(
                "{\"editor\": {\"format_on_save\": true}}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse nested config"
        );
        Check(config is not null && config.FormatOnSave, "config editor.formatOnSave");

        Check(
            HexalConfig.TryParse(
                "{\"formatOnSave\": false, \"editor\": {\"formatOnSave\": true}}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse config with override"
        );
        Check(config is not null && !config.FormatOnSave, "root formatOnSave takes precedence");

        Check(
            HexalConfig.TryParse("{}", "test.hexcfg", out config, out error),
            "parse empty config"
        );
        Check(
            config is not null
                && config.ShowDiffLines
                && !config.ShowGitIgnored
                && config.ConfirmQuitWithoutSaving
                && !config.SearchMatchCaseEnabled,
            "display settings have defaults"
        );
        Check(
            config is not null && config.KeepSidebarFocusOnOpen,
            "sidebar focus setting defaults on"
        );
        Check(
            config is not null
                && !config.TrimTrailingWhitespaceOnSave
                && config.SmartBackspace
                && config.AutoDetectIndentationOnStartup
                && !config.StartupBenchmarker
                && !config.SyntaxHighlightPickerPreview
                && config.DebugAdapter
                && config.CentralRepositoryUrl == HexalConfig.DefaultCentralRepositoryUrl,
            "whitespace settings have defaults"
        );

        Check(
            HexalConfig.TryParse(
                "{\"trimTrailingWhitespaceOnSave\": true, \"smartBackspace\": false}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse whitespace settings"
        );
        Check(
            config is not null && config.TrimTrailingWhitespaceOnSave && !config.SmartBackspace,
            "whitespace settings are read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"editor\": {\"trim_trailing_whitespace_on_save\": true, "
                    + "\"smart_backspace\": false}}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse nested whitespace settings"
        );
        Check(
            config is not null && config.TrimTrailingWhitespaceOnSave && !config.SmartBackspace,
            "nested whitespace settings are read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"autoDetectIndentationOnStartup\": false}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse auto-detect indentation setting"
        );
        Check(
            config is not null && !config.AutoDetectIndentationOnStartup,
            "auto-detect indentation setting is read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"editor\": {\"auto_detect_indentation_on_startup\": false}}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse nested auto-detect indentation setting"
        );
        Check(
            config is not null && !config.AutoDetectIndentationOnStartup,
            "nested auto-detect indentation setting is read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"startupBenchmarker\": true}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse startup benchmarker setting"
        );
        Check(
            config is not null && config.StartupBenchmarker,
            "startup benchmarker setting is read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"editor\": {\"startup_benchmarker\": true}}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse nested startup benchmarker setting"
        );
        Check(
            config is not null && config.StartupBenchmarker,
            "nested startup benchmarker setting is read"
        );

        Check(
            HexalConfig.TryParse("{\"debugAdapter\": false}", "test.hexcfg", out config, out error),
            "parse debug adapter setting"
        );
        Check(config is not null && !config.DebugAdapter, "debug adapter setting is read");

        Check(
            HexalConfig.TryParse(
                "{\"editor\": {\"debug_adapter\": false}}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse nested debug adapter setting"
        );
        Check(config is not null && !config.DebugAdapter, "nested debug adapter setting is read");

        Check(
            HexalConfig.TryParse(
                "{\"showDiffLines\": false, \"showGitIgnored\": true, "
                    + "\"searchMatchCaseEnabled\": true}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse display settings"
        );
        Check(
            config is not null
                && !config.ShowDiffLines
                && config.ShowGitIgnored
                && config.SearchMatchCaseEnabled,
            "display settings are read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"editor\": {\"search_match_case_enabled\": true}}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse nested search match case setting"
        );
        Check(
            config is not null && config.SearchMatchCaseEnabled,
            "nested search match case setting is read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"keepSidebarFocusOnOpen\": false}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse sidebar focus setting"
        );
        Check(
            config is not null && !config.KeepSidebarFocusOnOpen,
            "sidebar focus setting is read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"editor\": {\"keep_sidebar_focus_on_open\": false}}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse nested sidebar focus setting"
        );
        Check(
            config is not null && !config.KeepSidebarFocusOnOpen,
            "nested sidebar focus setting is read"
        );

        Check(
            HexalConfig.TryParse(
                "{\"confirmQuitWithoutSaving\": false}",
                "test.hexcfg",
                out config,
                out error
            ),
            "parse quit confirmation setting"
        );
        Check(
            config is not null && !config.ConfirmQuitWithoutSaving,
            "quit confirmation setting is read"
        );

        Check(
            !HexalConfig.TryParse("[]", "test.hexcfg", out config, out error),
            "reject non-object config"
        );
        Check(error.Length > 0, "config root error reported");

        using var directory = new TempDir();
        string configPath = directory.Path("test.hexcfg");
        Check(
            WriteBytes(configPath, "{\"formatOnSave\": true, \"custom\": \"kept\"}"u8),
            "write config for update"
        );
        Check(
            HexalConfig.UpdateDisplaySettings(
                configPath,
                showDiffLines: false,
                showGitIgnored: true,
                searchMatchCaseEnabled: true,
                out error
            ),
            "update display settings"
        );
        CheckEqual(error, string.Empty, "update error", "empty");

        HexalConfig updated = HexalConfig.Load(configPath, out error);
        CheckEqual(error, string.Empty, "updated config error", "empty");
        Check(updated.FormatOnSave, "update preserves formatOnSave");
        Check(!updated.ShowDiffLines, "update writes showDiffLines");
        Check(updated.ShowGitIgnored, "update writes showGitIgnored");
        Check(updated.SearchMatchCaseEnabled, "update writes searchMatchCaseEnabled");
        Check(
            File.ReadAllText(configPath).Contains("\"custom\": \"kept\"", StringComparison.Ordinal),
            "update preserves unknown settings"
        );
    }

    private static void TestFormatterRunner()
    {
        if (OperatingSystem.IsWindows())
        {
            return;
        }

        var config = new FormatterConfig
        {
            Command = "/bin/sh",
            Arguments = new[] { "-c", "tr a-z A-Z" },
            Input = "stdin",
            Output = "stdout",
        };

        FormatResult result = FormatterRunner.Run(
            config,
            "hello\n"u8.ToArray(),
            null,
            CancellationToken.None
        );
        Check(result.Ok, $"formatter succeeds ({result.Error})");
        CheckEqual(Text(result.Output), "HELLO\n", "formatter output", "HELLO\n");

        var missing = new FormatterConfig { Command = "hexal-formatter-that-does-not-exist" };
        FormatResult missingResult = FormatterRunner.Run(
            missing,
            "text"u8.ToArray(),
            null,
            CancellationToken.None
        );
        Check(!missingResult.Ok, "missing formatter fails");
        Check(missingResult.Error.Length > 0, "missing formatter reports error");
    }

    private static void TestLspPositions()
    {
        byte[] line = Encoding.UTF8.GetBytes("a😀b");
        CheckEqual(LspPositions.Utf16ColumnToByteOffset(line, 0), 0, "utf16 0", "0");
        CheckEqual(LspPositions.Utf16ColumnToByteOffset(line, 1), 1, "utf16 1", "1");
        CheckEqual(
            LspPositions.Utf16ColumnToByteOffset(line, 2),
            5,
            "utf16 inside surrogate rounds up",
            "5"
        );
        CheckEqual(LspPositions.Utf16ColumnToByteOffset(line, 3), 5, "utf16 3", "5");
        CheckEqual(LspPositions.Utf16ColumnToByteOffset(line, 4), 6, "utf16 4", "6");
        CheckEqual(LspPositions.Utf16ColumnToByteOffset(line, 99), 6, "utf16 past end clamps", "6");

        CheckEqual(LspPositions.ByteOffsetToUtf16Column(line, 1), 1, "byte 1", "1");
        CheckEqual(LspPositions.ByteOffsetToUtf16Column(line, 5), 3, "byte after emoji", "3");
        CheckEqual(LspPositions.ByteOffsetToUtf16Column(line, 6), 4, "byte past end", "4");
    }

    private sealed class FakeLspTransport : ILspServerTransport
    {
        public readonly List<string> Sent = new();
        public readonly Queue<string> Incoming = new();

        public bool IsAlive => true;

        public Action? Disconnected { get; set; }

        public void Send(string message) => Sent.Add(message);

        public bool TryReceive(out string message)
        {
            if (Incoming.Count > 0)
            {
                message = Incoming.Dequeue();
                return true;
            }

            message = string.Empty;
            return false;
        }

        public void Dispose() { }
    }

    private static void TestLspDebugReport()
    {
        string path = Path.GetTempFileName();
        File.WriteAllText(path, "class C {}\n");

        using var manager = new LspManager();
        var transport = new FakeLspTransport();
        bool started = manager.TryStart(
            new LspServerConfig
            {
                Name = "Fake",
                Transport = transport,
                LanguageId = "csharp",
            },
            path,
            "class C {}\n",
            Path.GetDirectoryName(path),
            out string startError
        );

        Check(started, "fake LSP started", startError);
        CheckEqual(transport.Sent.Count, 1, "fake transport send count", "initialize request");
        Check(
            manager.BuildDebugReport().Contains("Server: Fake", StringComparison.Ordinal),
            "debug report includes server name"
        );
        Check(
            manager.BuildDebugReport().Contains("-->", StringComparison.Ordinal),
            "debug report includes outbound traffic"
        );

        transport.Incoming.Enqueue("""{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}""");
        manager.Pump();

        string report = manager.BuildDebugReport();
        Check(
            report.Contains("<--", StringComparison.Ordinal),
            "debug report includes inbound traffic"
        );
        Check(
            report.Contains("initialize response received", StringComparison.Ordinal),
            "debug report includes lifecycle messages"
        );
        Check(
            report.Contains("initialized", StringComparison.Ordinal),
            "debug report includes initialized state"
        );

        manager.ClearLog();
        Check(
            manager.BuildDebugReport().Contains("log cleared", StringComparison.Ordinal),
            "debug report log can be cleared"
        );

        manager.Stop();
        string stopped = manager.BuildDebugReport();
        Check(
            stopped.Contains("State: stopped", StringComparison.Ordinal),
            "debug report shows stopped state"
        );
        Check(
            stopped.Contains("stopping Fake", StringComparison.Ordinal),
            "debug report logs server shutdown"
        );

        File.Delete(path);
    }

    private static void TestLspStartsForNewFile()
    {
        using var directory = new TempDir();
        string path = directory.Path("newfile.py");
        Check(!File.Exists(path), "new file does not exist before LSP start");

        using var manager = new LspManager();
        var transport = new FakeLspTransport();
        bool started = manager.TryStart(
            new LspServerConfig
            {
                Name = "FakeNew",
                Transport = transport,
                LanguageId = "python",
            },
            path,
            "print('new')\n",
            directory.Path("."),
            out string error
        );

        Check(started, $"new-file LSP started ({error})");
        CheckEqual(transport.Sent.Count, 1, "new-file LSP initialize count", "1");
        CheckEqual(manager.ServerName, "FakeNew", "new-file LSP name", "FakeNew");
        manager.Stop();
    }

    private static void TestLspDocumentSymbols()
    {
        using var directory = new TempDir();
        string path = directory.Path("symbols.lua");
        Check(
            WriteBytes(path, "local Widget = {}\nfunction Widget:Run() end\n"u8),
            "write symbols document"
        );

        using var manager = new LspManager();
        var transport = new FakeLspTransport();
        Check(
            manager.TryStart(
                new LspServerConfig
                {
                    Name = "Symbols",
                    Transport = transport,
                    LanguageId = "lua",
                },
                path,
                "local Widget = {}\n",
                directory.Path("."),
                out string error
            ),
            $"symbols LSP started ({error})"
        );

        transport.Incoming.Enqueue("""{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}""");
        manager.Pump();
        Check(manager.IsInitialized, "symbols LSP initialized");

        Check(manager.RequestDocumentSymbols(path), "request document symbols");
        string request = transport.Sent.Last(message =>
            message.Contains("textDocument/documentSymbol", StringComparison.Ordinal)
        );
        int requestId;
        using (
            System.Text.Json.JsonDocument document = System.Text.Json.JsonDocument.Parse(request)
        )
        {
            requestId = document.RootElement.GetProperty("id").GetInt32();
        }

        transport.Incoming.Enqueue(
            $$$$$$"""
            {"jsonrpc":"2.0","id":{{{{{{requestId}}}}}},"result":[
              {"name":"Widget","detail":"table","kind":5,
               "range":{"start":{"line":0,"character":0},"end":{"line":1,"character":1}},
               "selectionRange":{"start":{"line":0,"character":6},"end":{"line":0,"character":12}},
               "children":[
                 {"name":"Run","detail":"method","kind":6,
                  "range":{"start":{"line":1,"character":0},"end":{"line":1,"character":20}},
                  "selectionRange":{"start":{"line":1,"character":15},"end":{"line":1,"character":18}}}
               ]}
            ]}
            """
        );
        manager.Pump();

        bool seen = false;
        while (manager.TryDequeueEvent(out LspEvent lspEvent))
        {
            if (lspEvent.Kind != LspEventKind.DocumentSymbols)
            {
                continue;
            }

            seen = true;
            CheckEqual(lspEvent.DocumentSymbols.Count, 2, "document symbol count", "2");
            CheckEqual(
                lspEvent.DocumentSymbols[0].Name,
                "Widget",
                "document symbol name",
                "Widget"
            );
            CheckEqual(lspEvent.DocumentSymbols[0].Line, 0, "document symbol line", "0");
            CheckEqual(lspEvent.DocumentSymbols[0].Character, 6, "document symbol character", "6");
            CheckEqual(lspEvent.DocumentSymbols[0].Depth, 0, "document symbol depth", "0");
            CheckEqual(lspEvent.DocumentSymbols[1].Name, "Run", "nested symbol name", "Run");
            CheckEqual(lspEvent.DocumentSymbols[1].Line, 1, "nested symbol line", "1");
            CheckEqual(lspEvent.DocumentSymbols[1].Depth, 1, "nested symbol depth", "1");
        }
        Check(seen, "document symbol event received");

        Check(manager.RequestDocumentSymbols(path), "request flat symbols");
        request = transport.Sent.Last(message =>
            message.Contains("textDocument/documentSymbol", StringComparison.Ordinal)
        );
        using (
            System.Text.Json.JsonDocument document = System.Text.Json.JsonDocument.Parse(request)
        )
        {
            requestId = document.RootElement.GetProperty("id").GetInt32();
        }

        transport.Incoming.Enqueue(
            $$$$$$"""
            {"jsonrpc":"2.0","id":{{{{{{requestId}}}}}},"result":[
              {"name":"main","kind":12,
               "location":{"uri":"file:///tmp/symbols.lua",
                 "range":{"start":{"line":3,"character":4},"end":{"line":3,"character":8}}}}
            ]}
            """
        );
        manager.Pump();

        seen = false;
        while (manager.TryDequeueEvent(out LspEvent lspEvent))
        {
            if (lspEvent.Kind != LspEventKind.DocumentSymbols)
            {
                continue;
            }

            seen = true;
            CheckEqual(lspEvent.DocumentSymbols.Count, 1, "flat symbol count", "1");
            CheckEqual(lspEvent.DocumentSymbols[0].Name, "main", "flat symbol name", "main");
            CheckEqual(lspEvent.DocumentSymbols[0].Line, 3, "flat symbol line", "3");
            CheckEqual(lspEvent.DocumentSymbols[0].Character, 4, "flat symbol character", "4");
            CheckEqual(
                lspEvent.DocumentSymbols[0].Uri,
                "file:///tmp/symbols.lua",
                "flat symbol uri",
                "file:///tmp/symbols.lua"
            );
        }
        Check(seen, "flat document symbol event received");
    }

    private static void TestLspCompletions()
    {
        using var directory = new TempDir();
        string path = directory.Path("completion.cs");
        Check(WriteBytes(path, "class C { void M() { obj. } }\n"u8), "write completion document");

        using var manager = new LspManager();
        var transport = new FakeLspTransport();
        Check(
            manager.TryStart(
                new LspServerConfig
                {
                    Name = "Completion",
                    Transport = transport,
                    LanguageId = "csharp",
                },
                path,
                "class C { void M() { obj. } }\n",
                directory.Path("."),
                out string error
            ),
            $"completion LSP started ({error})"
        );

        transport.Incoming.Enqueue(
            """
            {"jsonrpc":"2.0","id":1,"result":{"capabilities":{
              "completionProvider":{"triggerCharacters":[".","<"]}}}}
            """
        );
        manager.Pump();
        Check(manager.IsInitialized, "completion LSP initialized");
        Check(manager.IsCompletionTriggerCharacter('.'), "completion dot trigger registered");
        Check(!manager.IsCompletionTriggerCharacter('x'), "completion non-trigger rejected");

        Check(manager.RequestCompletion(path, 0, 24, "."), "request completions");
        string request = transport.Sent.Last(message =>
            message.Contains("textDocument/completion", StringComparison.Ordinal)
        );
        int requestId;
        using (
            System.Text.Json.JsonDocument document = System.Text.Json.JsonDocument.Parse(request)
        )
        {
            requestId = document.RootElement.GetProperty("id").GetInt32();
        }

        transport.Incoming.Enqueue(
            $$$$$"""
            {"jsonrpc":"2.0","id":{{{{{requestId}}}}},"result":{"isIncomplete":false,"items":[
              {"label":"Name","kind":10,"detail":"string","insertText":"Name",
               "insertTextFormat":2,
               "textEdit":{"newText":"${1:name}",
                 "range":{"start":{"line":0,"character":23},
                          "end":{"line":0,"character":24}}}},
              {"label":"WriteLine","kind":2,"detail":"void",
               "insertText":"WriteLine(${1:value})","insertTextFormat":2}
            ]}}
            """
        );
        manager.Pump();

        bool seen = false;
        while (manager.TryDequeueEvent(out LspEvent lspEvent))
        {
            if (lspEvent.Kind != LspEventKind.Completions)
            {
                continue;
            }

            seen = true;
            CheckEqual(lspEvent.Completions.Count, 2, "completion item count", "2");
            CheckEqual(lspEvent.Completions[0].Label, "Name", "completion label", "Name");
            CheckEqual(
                lspEvent.Completions[0].InsertText,
                "${1:name}",
                "completion snippet text",
                "${1:name}"
            );
            CheckEqual(
                lspEvent.Completions[0].InsertTextFormat,
                2,
                "completion snippet format",
                "2"
            );
            Check(lspEvent.Completions[0].HasTextEdit, "completion text edit parsed");
            CheckEqual(lspEvent.Completions[0].StartCharacter, 23, "completion edit start", "23");
            CheckEqual(lspEvent.Completions[1].Detail, "void", "completion detail", "void");
        }
        Check(seen, "completion event received");
    }

    private static void TestDapLldbBreakpoint()
    {
        string? adapter = ExecutableFinder.Find("lldb-dap") ?? ExecutableFinder.Find("lldb-dap-18");
        string? compiler = ExecutableFinder.Find("cc") ?? ExecutableFinder.Find("gcc");
        if (adapter is null || compiler is null)
        {
            Console.Error.WriteLine(
                "lldb-dap or a C compiler unavailable; skipping DAP integration test"
            );
            return;
        }

        using var directory = new TempDir();
        string source = directory.Path("main.c");
        string executable = directory.Path("main");
        File.WriteAllText(
            source,
            """
            int main(void) {
                int value = 42;
                return value == 42 ? 0 : 1;
            }
            """,
            new UTF8Encoding(false)
        );

        Check(
            RunCommand(compiler, directory.Path("."), "-g", "-O0", "-o", executable, source),
            "compile C debugger target"
        );
        if (!File.Exists(executable))
        {
            return;
        }

        string fullSource = System.IO.Path.GetFullPath(source);
        var breakpoints = new Dictionary<string, SortedSet<int>>
        {
            [fullSource] = new SortedSet<int> { 2 },
        };
        var launch = new JsonObject
        {
            ["request"] = "launch",
            ["program"] = executable,
            ["cwd"] = directory.Path("."),
            ["stopOnEntry"] = false,
        };

        using var manager = new DapManager();
        Check(
            manager.TryStart(
                new DebuggerConfig { Adapter = "lldb-dap" },
                fullSource,
                directory.Path("."),
                launch,
                breakpoints,
                out string error
            ),
            $"start lldb-dap ({error})"
        );
        if (!manager.IsRunning)
        {
            return;
        }

        bool sawStopped = false;
        bool sawLocation = false;
        int stoppedLine = -1;
        string stoppedPath = string.Empty;
        string dapError = string.Empty;
        long deadline = Environment.TickCount64 + 15000;
        while (Environment.TickCount64 < deadline && (!sawStopped || !sawLocation))
        {
            while (manager.TryDequeueEvent(out DapEvent dapEvent))
            {
                switch (dapEvent.Kind)
                {
                    case DapEventKind.Stopped:
                        sawStopped = true;
                        break;
                    case DapEventKind.StoppedLocation:
                        sawLocation = true;
                        stoppedPath = dapEvent.Path;
                        stoppedLine = dapEvent.Line;
                        break;
                    case DapEventKind.Error:
                        dapError = dapEvent.Message;
                        break;
                    case DapEventKind.Terminated:
                        deadline = 0;
                        break;
                }
            }

            if (!manager.IsRunning || deadline == 0)
            {
                break;
            }
            Thread.Sleep(25);
        }

        if (!sawStopped || !sawLocation)
        {
            Console.Error.WriteLine(manager.BuildDebugReport());
        }
        Check(sawStopped, "DAP stopped at breakpoint");
        Check(sawLocation, $"DAP reported a stop location ({dapError})");
        CheckEqual(stoppedLine, 2, "DAP stopped line", "2");
        CheckEqual(
            stoppedPath.Length > 0 ? System.IO.Path.GetFullPath(stoppedPath) : string.Empty,
            fullSource,
            "DAP stopped path",
            fullSource
        );

        if (sawLocation)
        {
            Check(manager.RequestDebugVariables(), "DAP request variables");
            Check(manager.RequestEvaluate("value"), "DAP evaluate watch");

            bool sawScopes = false;
            bool sawVariables = false;
            bool sawWatch = false;
            string watchedValue = string.Empty;
            deadline = Environment.TickCount64 + 5000;
            while (Environment.TickCount64 < deadline && (!sawVariables || !sawWatch))
            {
                while (manager.TryDequeueEvent(out DapEvent dapEvent))
                {
                    switch (dapEvent.Kind)
                    {
                        case DapEventKind.Scopes:
                            sawScopes = dapEvent.Scopes.Count > 0;
                            break;
                        case DapEventKind.Variables:
                            sawVariables = dapEvent.ScopedVariables.Any(scope =>
                                scope.Variables.Any(variable =>
                                    string.Equals(variable.Name, "value", StringComparison.Ordinal)
                                    && variable.Value.Contains("42", StringComparison.Ordinal)
                                )
                            );
                            break;
                        case DapEventKind.WatchValue when dapEvent.Watch is { } watch:
                            sawWatch = string.Equals(
                                watch.Expression,
                                "value",
                                StringComparison.Ordinal
                            );
                            watchedValue = watch.Value;
                            break;
                        case DapEventKind.Error:
                            dapError = dapEvent.Message;
                            break;
                    }
                }

                if (
                    !manager.IsRunning
                    || (!sawVariables && !sawWatch && Environment.TickCount64 >= deadline)
                )
                {
                    break;
                }
                Thread.Sleep(25);
            }

            Check(sawScopes, "DAP returned scopes");
            Check(sawVariables, $"DAP returned local variables ({dapError})");
            Check(sawWatch, "DAP returned watch value");
            Check(
                watchedValue.Contains("42", StringComparison.Ordinal),
                "DAP watch value contains 42"
            );
        }

        if (sawStopped)
        {
            Check(manager.Continue(), "DAP continue");
            bool sawExit = false;
            deadline = Environment.TickCount64 + 10000;
            while (Environment.TickCount64 < deadline)
            {
                while (manager.TryDequeueEvent(out DapEvent dapEvent))
                {
                    if (dapEvent.Kind is DapEventKind.Exited or DapEventKind.Terminated)
                    {
                        sawExit = true;
                    }
                }

                if (sawExit || !manager.IsRunning)
                {
                    break;
                }
                Thread.Sleep(25);
            }
            Check(sawExit || !manager.IsRunning, "DAP program exits after continue");
        }

        manager.Stop();
    }

    private static void TestLanguageGrammars()
    {
        const string grammarJson = """
            {
                "id": "test",
                "name": "Test",
                "extensions": ["tst"],
                "keywords": ["fn", "let"],
                "controlStatements": ["if"],
                "types": ["int"],
                "constants": ["true"],
                "indentation": {
                    "style": "tabs",
                    "width": 8,
                    "tabWidth": 4
                },
                "comments": {
                    "line": "//",
                    "blockStart": "/*",
                    "blockEnd": "*/"
                },
                "strings": {
                    "quoteCharacters": ["\""],
                    "escapeCharacter": "\\"
                },
                "formatter": {
                    "command": "testfmt",
                    "arguments": ["--stdin", "{file}"],
                    "input": "stdin",
                    "output": "stdout",
                    "timeout": 2500,
                    "formatOnSave": true
                }
            }
            """;

        Check(
            LanguageDefinition.TryParse(
                grammarJson,
                "test.hexdef",
                out LanguageDefinition? language,
                out string parseError
            ),
            "parse grammar"
        );
        CheckEqual(parseError, string.Empty, "parse error", "empty");
        Check(language is not null, "grammar parsed");
        LanguageDefinition definition = language!;
        CheckEqual(definition.Id, "test", "grammar id", "test");
        CheckEqual(definition.Extensions[0], ".tst", "grammar extension", ".tst");
        Check(definition.Formatter is not null, "grammar formatter parsed");
        CheckEqual(definition.Formatter?.Command, "testfmt", "formatter command", "testfmt");
        CheckEqual(definition.Formatter?.Arguments.Count, 2, "formatter argument count", "2");
        CheckEqual(definition.Formatter?.TimeoutMs, 2500, "formatter timeout", "2500");
        CheckEqual(definition.FormatOnSave, true, "formatter formatOnSave", "true");
        Check(definition.Indentation is not null, "grammar indentation parsed");
        CheckEqual(definition.Indentation?.UseTabs, true, "grammar indentation uses tabs", "true");
        CheckEqual(definition.Indentation?.IndentWidth, 8, "grammar indentation width", "8");
        CheckEqual(definition.Indentation?.TabWidth, 4, "grammar tab width", "4");
        Check(definition.AutoIndent, "grammar auto indent defaults on");
        Check(definition.AutoClose, "grammar auto close defaults on");
        Check(!definition.TraditionalMarkupLanguage, "grammar is not markup by default");

        Check(
            LanguageDefinition.TryParse(
                """
                {
                    "id": "markup",
                    "autoIndent": false,
                    "autoClose": false,
                    "traditionalMarkupLanguage": true,
                    "indentAfter": [":", "=>"],
                    "unindentBefore": ["end"]
                }
                """,
                "markup.hexdef",
                out LanguageDefinition? markup,
                out parseError
            ),
            "parse markup grammar"
        );
        Check(markup is not null, "markup grammar parsed");
        Check(!markup!.AutoIndent, "markup auto indent opt-out");
        Check(!markup.AutoClose, "markup auto close opt-out");
        Check(markup.TraditionalMarkupLanguage, "markup mode enabled");
        CheckEqual(markup.IndentAfter.Count, 2, "indentAfter count", "2");
        Check(markup.IsUnindentKeyword("end"), "unindent keyword recognized");
        Check(!markup.IsUnindentKeyword("ended"), "partial unindent keyword rejected");

        Check(
            LanguageDefinition.TryParse(
                """
                {
                    "id": "debuggable",
                    "debugger": {
                        "adapter": "lldb-dap",
                        "arguments": ["--stdio"],
                        "configuration": {
                            "request": "launch",
                            "program": "${executable}",
                            "cwd": "${workspace}"
                        }
                    }
                }
                """,
                "debuggable.hexdef",
                out LanguageDefinition? debuggable,
                out parseError
            ),
            "parse debugger grammar"
        );
        Check(debuggable?.Debugger is not null, "debugger metadata parsed");
        CheckEqual(debuggable?.Debugger?.Adapter, "lldb-dap", "debug adapter", "lldb-dap");
        CheckEqual(debuggable?.Debugger?.Arguments.Count, 1, "debug adapter argument count", "1");
        CheckEqual(
            debuggable?.Debugger?.Configuration["program"]?.GetValue<string>(),
            "${executable}",
            "debug program placeholder",
            "${executable}"
        );

        using var directory = new TempDir();
        string grammarPath = directory.Path("test.hexdef");
        Check(WriteBytes(grammarPath, Encoding.UTF8.GetBytes(grammarJson)), "write grammar file");

        var registry = new LanguageRegistry();
        registry.LoadDirectory(directory.Path("."));
        CheckEqual(registry.Errors.Count, 0, "registry errors", "0");
        Check(
            ReferenceEquals(registry.Detect("sample.tst"), registry.Find("test")),
            "extension detection"
        );

        var bundled = LanguageRegistry.LoadUserGrammars();
        Check(bundled.Errors.Count == 0, "bundled grammars load without errors");
        Check(bundled.Find("ruby") is not null, "bundled ruby grammar loads");
        Check(bundled.Find("rust") is not null, "bundled rust grammar loads");
        Check(
            ReferenceEquals(bundled.Detect("main.rs"), bundled.Find("rust")),
            "bundled rust extension detection"
        );
        Check(bundled.Find("lua")?.IsUnindentKeyword("end") == true, "bundled lua unindents end");
        Check(bundled.Find("ruby")?.IsUnindentKeyword("end") == true, "bundled ruby unindents end");

        string[] addedLanguages =
        {
            "nix",
            "perl",
            "raku",
            "julia",
            "fsharp",
            "blade",
            "jinja",
            "nunjucks",
            "gotempl",
            "meson",
            "matlab",
            "godot-tscn",
            "rebol",
            "red",
            "diff",
            "mlir",
            "elm",
            "git-commit",
            "graphql",
            "haskell",
            "scheme",
            "supercollider",
            "ocaml",
        };
        foreach (string addedLanguage in addedLanguages)
        {
            Check(
                bundled.Find(addedLanguage) is not null,
                $"bundled {addedLanguage} grammar loads"
            );
        }

        LanguageDefinition? csharpLanguage = bundled.Find("csharp");
        Check(
            csharpLanguage?.Formatter?.Command == "csharpier"
                && csharpLanguage.Formatter.Arguments.Count == 3
                && csharpLanguage.Formatter.Arguments[0] == "format"
                && csharpLanguage.Formatter.Arguments[1] == "--stdin-path"
                && csharpLanguage.Formatter.Arguments[2] == "{path}"
                && csharpLanguage.Formatter.UsesStdin
                && !csharpLanguage.Formatter.WritesFile,
            "csharpier formats C# through stdin/stdout"
        );

        Check(
            ReferenceEquals(bundled.Detect("file.fs"), bundled.Find("fsharp")),
            "fsharp owns .fs"
        );
        Check(
            ReferenceEquals(bundled.Detect("shader.glsl"), bundled.Find("glsl")),
            "glsl detection still works"
        );
        Check(
            ReferenceEquals(bundled.Detect("view.blade.php"), bundled.Find("blade")),
            "compound blade extension detection"
        );
        Check(
            ReferenceEquals(bundled.Detect("COMMIT_EDITMSG"), bundled.Find("git-commit")),
            "git commit file name detection"
        );
        Check(
            ReferenceEquals(bundled.Detect("meson.build"), bundled.Find("meson")),
            "meson file name detection"
        );
        Check(
            ReferenceEquals(bundled.Detect("script.nix"), bundled.Find("nix")),
            "nix extension detection"
        );
        Check(
            ReferenceEquals(bundled.Detect("Dockerfile"), bundled.Find("dockerfile")),
            "Dockerfile file name detection"
        );
        Check(
            ReferenceEquals(bundled.Detect("/repo/Containerfile"), bundled.Find("dockerfile")),
            "Containerfile file name detection"
        );

        var diffTokens = new List<SyntaxToken>();
        LanguageDefinition? dockerfileLanguage = bundled.Find("dockerfile");
        Check(dockerfileLanguage is not null, "dockerfile language available");
        if (dockerfileLanguage is not null)
        {
            diffTokens.Clear();
            SyntaxHighlighter.GetLineTokens(
                dockerfileLanguage,
                "FROM alpine:3.20"u8,
                false,
                diffTokens,
                out _
            );
            Check(
                diffTokens.Count >= 1
                    && diffTokens[0].Kind == SyntaxTokenKind.Keyword
                    && diffTokens[0].Start == 0
                    && diffTokens[0].Length == 4,
                "dockerfile instructions are highlighted"
            );
        }
        LanguageDefinition? diffLanguage = bundled.Find("diff");
        Check(diffLanguage is not null, "diff language available");
        if (diffLanguage is not null)
        {
            diffTokens.Clear();
            SyntaxHighlighter.GetLineTokens(diffLanguage, "+added"u8, false, diffTokens, out _);
            Check(
                diffTokens.Count == 1 && diffTokens[0].Kind == SyntaxTokenKind.Added,
                "diff added line is highlighted"
            );
            diffTokens.Clear();
            SyntaxHighlighter.GetLineTokens(diffLanguage, "-removed"u8, false, diffTokens, out _);
            Check(
                diffTokens.Count == 1 && diffTokens[0].Kind == SyntaxTokenKind.Deleted,
                "diff deleted line is highlighted"
            );
        }

        LanguageDefinition? commitLanguage = bundled.Find("git-commit");
        Check(commitLanguage is not null, "git commit language available");
        if (commitLanguage is not null)
        {
            diffTokens.Clear();
            SyntaxHighlighter.GetLineTokens(
                commitLanguage,
                "feat: add some thing"u8,
                false,
                diffTokens,
                out _
            );
            Check(
                diffTokens.Count == 1
                    && diffTokens[0].Kind == SyntaxTokenKind.Keyword
                    && diffTokens[0].Start == 0
                    && diffTokens[0].Length == 5,
                "git commit prefix including colon is highlighted"
            );
        }

        string sourcePath = directory.Path("sample.tst");
        const string source =
            "fn main() {\n  let x = 42;\n  // comment\n  /* block\n     still */\n  int y = true;\n}\n";
        Check(WriteBytes(sourcePath, Encoding.UTF8.GetBytes(source)), "write source");
        Document? document = Document.Open(sourcePath, out string openError);
        Check(document is not null, $"open source ({openError})");
        if (document is null)
        {
            return;
        }

        var highlighter = new SyntaxHighlighter();
        highlighter.SetDocument(document, definition);
        var tokens = new List<SyntaxToken>();

        highlighter.GetLineTokens(0, document.LineText(0), tokens);
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Keyword), "keyword token");
        Check(
            tokens.Any(token => token.Kind == SyntaxTokenKind.ControlStatement) == false,
            "no control token on line 0"
        );

        highlighter.GetLineTokens(1, document.LineText(1), tokens);
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Number), "number token");
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Keyword), "line 1 keyword token");

        highlighter.GetLineTokens(2, document.LineText(2), tokens);
        Check(tokens.Count == 1 && tokens[0].Kind == SyntaxTokenKind.Comment, "line comment token");

        highlighter.GetLineTokens(3, document.LineText(3), tokens);
        Check(
            tokens.Any(token => token.Kind == SyntaxTokenKind.Comment),
            "block comment start token"
        );

        highlighter.GetLineTokens(4, document.LineText(4), tokens);
        Check(
            tokens.Count == 1 && tokens[0].Kind == SyntaxTokenKind.Comment,
            "continued block comment token"
        );

        highlighter.GetLineTokens(5, document.LineText(5), tokens);
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Type), "type token");
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Constant), "constant token");
    }

    private static void TestThemes()
    {
        const string json = """
            {
                "id": "test-theme",
                "name": "Test Theme",
                "description": "Theme parser fixture.",
                "author": "Test Author",
                "license": "MIT",
                "colors": {
                    "foreground": "#112233",
                    "background": "black",
                    "muted": 245,
                    "accent": "bright-cyan",
                    "selection": { "fg": "#ffffff", "bg": 4 },
                    "syntax": { "keyword": "#ff00aa" }
                }
            }
            """;

        Check(
            Theme.TryParse(json, "test-theme.json", out Theme? theme, out string error),
            $"parse theme ({error})"
        );
        Check(theme is not null, "theme parsed");
        if (theme is null)
        {
            return;
        }

        CheckEqual(theme.Id, "test-theme", "theme id", "test-theme");
        CheckEqual(theme.Name, "Test Theme", "theme name", "Test Theme");
        CheckEqual(theme.Author, "Test Author", "theme author", "Test Author");
        CheckEqual(theme.License, "MIT", "theme license", "MIT");
        Check(Theme.IsTrueColor(theme.Foreground), "hex foreground is true colour");
        CheckEqual(Theme.Red(theme.Foreground), (byte)0x11, "theme red", "0x11");
        CheckEqual(Theme.Green(theme.Foreground), (byte)0x22, "theme green", "0x22");
        CheckEqual(Theme.Blue(theme.Foreground), (byte)0x33, "theme blue", "0x33");
        CheckEqual(theme.Background, 0u, "named background", "0");
        CheckEqual(theme.Muted, 245u, "numeric muted", "245");
        CheckEqual(theme.Accent, 14u, "named accent", "14");
        CheckEqual(theme.SelectionForeground, Theme.Rgb(255, 255, 255), "selection fg", "#ffffff");
        CheckEqual(theme.SelectionBackground, 4u, "selection bg", "4");
        CheckEqual(theme.Keyword, Theme.Rgb(255, 0, 170), "keyword", "#ff00aa");

        Check(
            !Theme.TryParse(
                "{\"colors\": {\"foreground\": \"not-a-colour\"}}",
                "bad.json",
                out _,
                out error
            ),
            "reject invalid colour"
        );
        Check(error.Length > 0, "invalid colour reports error");

        using var directory = new TempDir();
        File.WriteAllText(directory.Path("one.json"), json);
        File.WriteAllText(directory.Path("two.json"), "{\"id\": \"two\", \"name\": \"Two\"}");
        var registry = new ThemeRegistry();
        registry.LoadDirectory(directory.Path("."));
        CheckEqual(registry.Themes.Count, 2, "theme registry count", "2");
        Check(registry.Find("two") is not null, "theme registry find");
        Check(registry.Find("missing") is null, "theme registry missing");
    }

    private static void TestPluginConfigStore()
    {
        using var directory = new TempDir();
        string path = directory.Path("plugin.json");
        var store = new PluginConfigStore(path);
        store.Set("answer", 42);
        store.Set("nested.name", "hexal");
        Check(store.Save(out string error), $"save plugin config ({error})");

        var loaded = new PluginConfigStore(path);
        Check(loaded.TryLoad(out error), $"load plugin config ({error})");
        CheckEqual(Convert.ToInt64(loaded.Get("answer")), 42L, "config answer", "42");
        CheckEqual(loaded.Get("nested.name") as string, "hexal", "nested config value", "hexal");
        CheckEqual(
            loaded.Get("missing", "fallback") as string,
            "fallback",
            "config fallback",
            "fallback"
        );
    }

    private static void TestPluginMetadataAndToggle()
    {
        using var directory = new TempDir();
        string plugins = directory.Path("plugins");
        string plugin = System.IO.Path.Combine(plugins, "example");
        Directory.CreateDirectory(plugin);

        File.WriteAllText(
            System.IO.Path.Combine(plugin, "plugin.toml"),
            """
            name = "Example"
            version = "1.2.3"
            description = "Example plugin"
            author = "Test Author"
            license = "MIT"
            """,
            new UTF8Encoding(false)
        );
        File.WriteAllText(
            System.IO.Path.Combine(plugin, "init.lua"),
            """
            hexal.commands.register("hello", function()
                hexal.ui.message("HI")
            end, "Say hi")
            """,
            new UTF8Encoding(false)
        );

        var host = new FakeHost();
        using (var manager = new LuaPluginManager(host, plugins))
        {
            manager.LoadAll();
            CheckEqual(manager.Errors.Count, 0, "plugin metadata errors", "0");
            CheckEqual(manager.PluginCount, 1, "plugin metadata count", "1");

            PluginInfo info = manager.Plugins.Single();
            CheckEqual(info.Id, "example", "plugin id", "example");
            CheckEqual(info.Version, "1.2.3", "plugin version", "1.2.3");
            CheckEqual(info.Description, "Example plugin", "plugin description", "Example plugin");
            CheckEqual(info.Author, "Test Author", "plugin author", "Test Author");
            CheckEqual(info.License, "MIT", "plugin license", "MIT");
            Check(info.Enabled, "plugin starts enabled");
            Check(info.Running, "plugin starts running");
            Check(info.Commands.Contains("hello"), "plugin command listed");
            Check(manager.HasCommand("hello"), "plugin command registered");

            Check(
                manager.SetPluginEnabled("example", false, out string error),
                $"disable plugin ({error})"
            );
            CheckEqual(manager.PluginCount, 0, "disabled plugin unloaded", "0");
            Check(!manager.HasCommand("hello"), "disabled plugin command removed");
            info = manager.Plugins.Single();
            Check(!info.Enabled, "plugin reports disabled");
            Check(!info.Running, "disabled plugin not running");

            Check(manager.SetPluginEnabled("example", true, out error), $"enable plugin ({error})");
            CheckEqual(manager.PluginCount, 1, "enabled plugin loaded", "1");
            Check(manager.HasCommand("hello"), "enabled plugin command restored");
        }

        string statePath = directory.Path("disabled-plugins");
        using (var manager = new LuaPluginManager(host, plugins, statePath))
        {
            manager.LoadAll();
            Check(
                manager.SetPluginEnabled("example", false, out string error),
                $"disable plugin for state ({error})"
            );
        }

        using (var manager = new LuaPluginManager(host, plugins, statePath))
        {
            manager.LoadAll();
            CheckEqual(manager.PluginCount, 0, "disabled state persists", "0");
            Check(!manager.Plugins.Single().Enabled, "persisted plugin reports disabled");
        }
    }

    private static void TestPluginReload()
    {
        using var directory = new TempDir();
        string plugins = directory.Path("plugins");
        string plugin = System.IO.Path.Combine(plugins, "reloadable");
        Directory.CreateDirectory(plugin);

        void WritePlugin(string command)
        {
            File.WriteAllText(
                System.IO.Path.Combine(plugin, "plugin.toml"),
                """
                name = "Reloadable"
                version = "1.0"
                """,
                new UTF8Encoding(false)
            );
            File.WriteAllText(
                System.IO.Path.Combine(plugin, "init.lua"),
                $"""
                hexal.commands.register("{command}", function() end, "Test command")
                """,
                new UTF8Encoding(false)
            );
        }

        WritePlugin("first");

        var host = new FakeHost();
        using var manager = new LuaPluginManager(host, plugins);
        manager.LoadAll();
        Check(manager.HasCommand("first"), "reload initial command registered");
        Check(!manager.HasCommand("second"), "reload second command absent");

        WritePlugin("second");
        Check(manager.ReloadPlugin("reloadable", out string error), $"reload plugin ({error})");
        Check(manager.HasCommand("second"), "reloaded command registered");
        Check(!manager.HasCommand("first"), "reloaded old command removed");

        WritePlugin("third");
        Check(manager.ReloadAll(out error), $"reload all plugins ({error})");
        Check(manager.HasCommand("third"), "reload all command registered");
        Check(!manager.HasCommand("second"), "reload all old command removed");
    }

    private static void TestPluginInstaller()
    {
        using var directory = new TempDir();
        string repository = directory.Path("repository");
        string plugin = System.IO.Path.Combine(repository, "demo");
        Directory.CreateDirectory(plugin);
        File.WriteAllText(
            System.IO.Path.Combine(plugin, "plugin.toml"),
            """
            name = "Demo"
            version = "1.0"
            """,
            new UTF8Encoding(false)
        );
        File.WriteAllText(
            System.IO.Path.Combine(plugin, "init.lua"),
            "hexal.commands.register(\"demo\", function() end)\n",
            new UTF8Encoding(false)
        );

        if (!RunCommand("git", repository, "init", "-q"))
        {
            Console.Error.WriteLine("git unavailable; skipping plugin installer test");
            return;
        }

        RunCommand("git", repository, "config", "user.email", "tests@example.com");
        RunCommand("git", repository, "config", "user.name", "Hexal Tests");
        Check(
            RunCommand("git", repository, "add", "demo/plugin.toml", "demo/init.lua"),
            "git add plugin"
        );
        Check(RunCommand("git", repository, "commit", "-qm", "plugin"), "git commit plugin");

        string plugins = directory.Path("plugins");
        Check(
            PluginInstaller.TryInstall(
                "demo",
                repository,
                plugins,
                out string installed,
                out string error
            ),
            $"install plugin ({error})"
        );
        Check(Directory.Exists(installed), "installed plugin directory exists");
        Check(
            File.Exists(System.IO.Path.Combine(installed, "plugin.toml")),
            "installed plugin manifest exists"
        );
        Check(
            File.Exists(System.IO.Path.Combine(installed, "init.lua")),
            "installed plugin entry exists"
        );
        Check(
            !Directory.Exists(System.IO.Path.Combine(installed, ".git")),
            "installed plugin has no repository metadata"
        );

        Check(
            !PluginInstaller.TryInstall(
                "demo",
                repository,
                plugins,
                out _,
                out string duplicateError
            ),
            "duplicate plugin install is rejected"
        );
        Check(
            duplicateError.Contains("already installed", StringComparison.OrdinalIgnoreCase),
            "duplicate install error is descriptive"
        );

        Check(
            !PluginInstaller.TryInstall("../demo", repository, plugins, out _, out _),
            "path traversal plugin name is rejected"
        );
    }

    private static void TestPluginProcessAndPanel()
    {
        using var directory = new TempDir();
        string plugins = directory.Path("plugins");
        string plugin = System.IO.Path.Combine(plugins, "api");
        Directory.CreateDirectory(plugin);

        string processCall = OperatingSystem.IsWindows()
            ? "local result = hexal.exec.run(\"cmd\", { \"/c\", \"echo\", \"hello\" })"
            : "local result = hexal.exec.run(\"echo\", { \"hello\" })";

        File.WriteAllText(
            System.IO.Path.Combine(plugin, "plugin.toml"),
            """
            name = "API"
            version = "1.0"
            """,
            new UTF8Encoding(false)
        );
        File.WriteAllText(
            System.IO.Path.Combine(plugin, "init.lua"),
            $$"""
            {{processCall}}
            assert(result.ok, result.error)
            assert(result.stdout:find("hello", 1, true) ~= nil, result.stdout)
            assert(hexal.lsp.missing_api_key == nil,
                "missing lazy LSP keys should return nil")

            local panel = hexal.ui.panel({
                title = "Test",
                lines = {
                    { segments = { { text = "one", style = "accent" } } },
                    { segments = { { text = "two", style = "diff_added" } },
                      selectable = false },
                },
                selected = 1,
            }, function(key, selected)
                if key == "enter" then
                    return true
                end
                return false
            end)

            panel:set_title("Changed")
            panel:set_footer("footer")
            panel:set_selectable(true)
            panel:set_selected(2)
            panel:set_lines({
                { segments = { { text = "three", style = "muted" } } },
            })
            panel:close()
            hexal.ui.message("PROCESS_PANEL_OK")
            """,
            new UTF8Encoding(false)
        );

        var host = new FakeHost();
        using var manager = new LuaPluginManager(host, plugins);
        manager.LoadAll();

        if (manager.Errors.Count > 0)
        {
            Console.Error.WriteLine(string.Join("\n", manager.Errors));
        }
        CheckEqual(manager.Errors.Count, 0, "plugin API errors", "0");
        Check(host.Messages.Contains("PROCESS_PANEL_OK"), "plugin process and panel API completed");
        CheckEqual(host.PanelLines.Count, 1, "plugin panel line update", "1");
        CheckEqual(
            host.PanelLines[0].Segments[0].Text,
            "three",
            "plugin panel segment text",
            "three"
        );
    }

    private static void TestLazyPluginLoading()
    {
        using var directory = new TempDir();
        string plugins = directory.Path("plugins");
        string plugin = System.IO.Path.Combine(plugins, "lazy");
        Directory.CreateDirectory(plugin);

        File.WriteAllText(
            System.IO.Path.Combine(plugin, "plugin.toml"),
            """
            name = "Lazy"
            version = "1.0"
            lazy = true
            language = "test"
            """,
            new UTF8Encoding(false)
        );
        File.WriteAllText(
            System.IO.Path.Combine(plugin, "init.lua"),
            """
            hexal.ui.message("LAZY_STARTED")
            hexal.events.on("buffer_opened", function(event)
                hexal.ui.message("LAZY_OPENED:" .. tostring(event.language))
            end)
            """,
            new UTF8Encoding(false)
        );

        var host = new FakeHost();
        using var manager = new LuaPluginManager(host, plugins);
        manager.LoadAll();

        CheckEqual(manager.PluginCount, 0, "lazy plugin not started", "0");
        CheckEqual(manager.DeferredPluginCount, 1, "lazy plugin deferred count", "1");

        manager.FireEvent(
            "buffer_opened",
            new Dictionary<string, object?> { ["language"] = "other" }
        );
        CheckEqual(manager.PluginCount, 0, "lazy plugin ignores other language", "0");
        CheckEqual(manager.DeferredPluginCount, 1, "lazy plugin remains deferred", "1");

        manager.FireEvent(
            "buffer_opened",
            new Dictionary<string, object?> { ["language"] = "test" }
        );
        CheckEqual(manager.PluginCount, 1, "lazy plugin started on language", "1");
        CheckEqual(manager.DeferredPluginCount, 0, "lazy plugin no longer deferred", "0");
        Check(host.Messages.Contains("LAZY_STARTED"), "lazy plugin init script ran");
        Check(host.Messages.Contains("LAZY_OPENED:test"), "lazy plugin received the opening event");
    }

    private static void TestGitLogPlugin()
    {
        string source = System.IO.Path.Combine(
            AppContext.BaseDirectory,
            "data",
            "plugins",
            "gitlog"
        );
        Check(Directory.Exists(source), "bundled git log plugin exists");
        if (!Directory.Exists(source))
        {
            return;
        }

        using var directory = new TempDir();
        string repo = directory.Path("repo");
        Directory.CreateDirectory(repo);
        if (!RunCommand("git", repo, "init", "-q"))
        {
            Console.Error.WriteLine("git unavailable; skipping git log integration test");
            return;
        }

        RunCommand("git", repo, "config", "user.email", "tests@example.com");
        RunCommand("git", repo, "config", "user.name", "Hexal Tests");

        string tracked = System.IO.Path.Combine(repo, "file.txt");
        File.WriteAllText(tracked, "one\ntwo\nthree\n", new UTF8Encoding(false));
        Check(RunCommand("git", repo, "add", "file.txt"), "git add first commit");
        Check(RunCommand("git", repo, "commit", "-qm", "first commit"), "git create first commit");

        File.WriteAllText(tracked, "one\nTHREE\n", new UTF8Encoding(false));
        Check(RunCommand("git", repo, "add", "file.txt"), "git add second commit");
        Check(
            RunCommand("git", repo, "commit", "-qm", "second commit"),
            "git create second commit"
        );
        RunCommand("git", repo, "tag", "v1.0");
        RunCommand("git", repo, "branch", "feature");

        string plugins = directory.Path("plugins");
        string plugin = System.IO.Path.Combine(plugins, "gitlog");
        Directory.CreateDirectory(plugin);
        foreach (string file in Directory.GetFiles(source))
        {
            File.Copy(file, System.IO.Path.Combine(plugin, System.IO.Path.GetFileName(file)));
        }

        var host = new FakeHost { WorkspaceRoot = repo };
        using var manager = new LuaPluginManager(host, plugins);
        manager.LoadAll();

        if (manager.Errors.Count > 0)
        {
            Console.Error.WriteLine(string.Join("\n", manager.Errors));
        }
        CheckEqual(manager.Errors.Count, 0, "git log plugin errors", "0");
        Check(manager.HasCommand("gl"), "git log registers :gl");
        Check(
            manager.TryExecuteCommand("gl", Array.Empty<string>(), out string error),
            $"run git log command ({error})"
        );
        CheckEqual(error, string.Empty, "git log command error", "empty");
        Check(host.PanelLines.Count > 0, "git log opens a panel");

        bool sawTag = false;
        bool sawBranch = false;
        bool sawAuthor = false;
        foreach (PluginPanelLine line in host.PanelLines)
        {
            string text = string.Concat(line.Segments.Select(segment => segment.Text));
            sawTag |= text.Contains("v1.0", StringComparison.Ordinal);
            sawBranch |= text.Contains("feature", StringComparison.Ordinal);
            sawAuthor |= text.Contains("Hexal Tests", StringComparison.Ordinal);
        }
        Check(sawTag, "git log shows tags");
        Check(sawBranch, "git log shows branches");
        Check(sawAuthor, "git log shows authors");

        Check(host.PanelKeyHandler is not null, "git log panel has a key handler");
        if (host.PanelKeyHandler is not null)
        {
            Check(host.PanelKeyHandler("enter", 1), "git log opens a commit diff");

            bool added = false;
            bool deleted = false;
            foreach (PluginPanelLine line in host.PanelLines)
            {
                foreach (PluginPanelSegment segment in line.Segments)
                {
                    added |= string.Equals(
                        segment.Style,
                        "diff_added",
                        StringComparison.OrdinalIgnoreCase
                    );
                    deleted |= string.Equals(
                        segment.Style,
                        "diff_deleted",
                        StringComparison.OrdinalIgnoreCase
                    );
                }
            }

            Check(added, "git diff highlights insertions");
            Check(deleted, "git diff highlights deletions");
        }
    }

    private static bool RunCommand(string command, string directory, params string[] arguments)
    {
        try
        {
            var startInfo = new System.Diagnostics.ProcessStartInfo
            {
                FileName = command,
                WorkingDirectory = directory,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                CreateNoWindow = true,
            };
            foreach (string argument in arguments)
            {
                startInfo.ArgumentList.Add(argument);
            }

            using var process = System.Diagnostics.Process.Start(startInfo);
            if (process is null)
            {
                return false;
            }

            process.StandardOutput.ReadToEnd();
            process.StandardError.ReadToEnd();
            process.WaitForExit();
            return process.ExitCode == 0;
        }
        catch (Exception ex)
            when (ex
                    is System.ComponentModel.Win32Exception
                        or InvalidOperationException
                        or IOException
                        or UnauthorizedAccessException
            )
        {
            return false;
        }
    }

    private static void TestLazyCSharpPlugin()
    {
        string source = System.IO.Path.Combine(
            AppContext.BaseDirectory,
            "data",
            "plugins",
            "csharp"
        );
        Check(Directory.Exists(source), "bundled csharp plugin exists");
        if (!Directory.Exists(source))
        {
            return;
        }

        using var directory = new TempDir();
        string plugins = directory.Path("plugins");
        string plugin = System.IO.Path.Combine(plugins, "csharp");
        Directory.CreateDirectory(plugin);
        foreach (string file in Directory.GetFiles(source))
        {
            File.Copy(file, System.IO.Path.Combine(plugin, System.IO.Path.GetFileName(file)));
        }

        var host = new FakeHost();
        using var manager = new LuaPluginManager(host, plugins);
        manager.LoadAll();

        CheckEqual(manager.Errors.Count, 0, "csharp plugin errors", "0");
        CheckEqual(manager.Coordinators.Count, 0, "csharp LSP is lazy at startup", "0");

        manager.FireEvent(
            "buffer_opened",
            new Dictionary<string, object?> { ["language"] = "csharp" }
        );
        CheckEqual(manager.Coordinators.Count, 1, "csharp LSP starts for csharp buffers", "1");
        CheckEqual(manager.Coordinators[0].Servers.Count, 1, "csharp server registered", "1");
    }

    private static void TestLuaLspRuntime()
    {
        using var directory = new TempDir();
        string plugins = directory.Path("plugins");
        string plugin = System.IO.Path.Combine(plugins, "test-lsp");
        Directory.CreateDirectory(plugin);

        const string script = """
            local lsp = require("hexal.lsp")
            local diagnostics = require("hexal.lsp.diagnostics")
            local types = require("hexal.lsp.types")
            local json = require("hexal.json")
            local uri = require("hexal.lsp.uri")

            assert(uri.is_file("file:///tmp/x"), "dot module function failed")
            assert(uri:is_file("file:///tmp/x"), "colon module function failed")
            local server = lsp.create_server({
                name = "test-lsp",
                version = "1.0.0",
                filetypes = { "test" },
                extensions = { ".tst" },
                root_markers = { ".git" },
            })
            assert(server, "server was not created")
            local function step(name, fn)
                local ok, err = pcall(fn)
                assert(ok, name .. ": " .. tostring(err))
            end
            step("start", function() assert(server:start()) end)
            step("state", function() assert(server:state() == "running") end)
            step("caps", function() assert(server:capabilities({ hover = true, completion = true })) end)

            step("on", function()
                server:on("initialize", function(params, ctx)
                    return {
                        capabilities = server:advertised_capabilities(),
                        serverInfo = { name = "test-lsp" },
                    }
                end)
            end)

            step("register", function()
                server:register("textDocument/hover", function(params, ctx)
                    local doc = ctx.documents.get(params.textDocument.uri)
                    assert(doc, "document missing")
                    assert(doc.text == "hello", "document text")
                    assert(doc:word_at({ line = 0, character = 1 }) == "hello")
                    return {
                        contents = { kind = "markdown", value = "hover-doc" },
                    }
                end)
                server:register("textDocument/definition", function(params, ctx)
                    return {
                        uri = "file:///test.tst",
                        range = {
                            start = { line = 0, character = 2 },
                            ["end"] = { line = 0, character = 5 },
                        },
                    }
                end)
            end)

            local test = require("hexal.testing.lsp")
            local client = test.create_client(server)
            assert(client, "test client missing")
            assert(client:initialize(), "initialize failed")
            assert(client:open_document("file:///test.tst", "test", "hello"))
            local result = client:request("textDocument/hover", {
                textDocument = { uri = "file:///test.tst" },
                position = { line = 0, character = 1 },
            })
            assert(result and result.contents and result.contents.value == "hover-doc",
                "hover result missing")
            local definition = client:request("textDocument/definition", {
                textDocument = { uri = "file:///test.tst" },
                position = { line = 0, character = 1 },
            })
            assert(definition and definition.uri == "file:///test.tst" and
                definition.range.start.character == 2, "definition result missing")

            diagnostics.publish("file:///test.tst", {
                {
                    range = {
                        start = { line = 0, character = 0 },
                        ["end"] = { line = 0, character = 5 },
                    },
                    severity = diagnostics.severity.ERROR,
                    message = "bad",
                },
            })
            local items = diagnostics.get("file:///test.tst")
            assert(#items == 1, "diagnostics were not stored")

            assert(type(json.encode({ a = 1 })) == "string")
            assert(uri.to_path(uri.from_path("/tmp/hexal-test")) == "/tmp/hexal-test")
            assert(types.position(1, 2).line == 1)
            assert(type(require("hexal.async").run) == "function")
            assert(type(require("hexal.fs").read_file) == "function")
            assert(type(require("hexal.log").info) == "function")
            assert(type(require("hexal.index").create) == "function")

            local protocol = require("hexal.lsp.protocol")
            server:register("example/reject", function(params, ctx)
                return ctx:reject(protocol.error_codes.INVALID_PARAMS, "bad params")
            end)
            local rejected = client:request_raw("example/reject", {})
            assert(not rejected.ok and rejected.error:find("bad params"),
                "reject " .. tostring(rejected.ok) .. " " .. tostring(rejected.error))

            server:register("textDocument/semanticTokens/full", function(params, ctx)
                return {
                    { line = 0, character = 0, length = 5, type = "variable" },
                }
            end)
            local tokens = client:request("textDocument/semanticTokens/full", {
                textDocument = { uri = "file:///test.tst" },
            })
            assert(tokens and #tokens.data == 5 and tokens.data[3] == 5)

            local completion = require("hexal.lsp.completion")
            local item = completion.item({
                label = "greet",
                kind = "FUNCTION",
                insert_text = "greet()",
            })
            assert(item.label == "greet" and item.insertText == "greet()")
            assert(item.kind == completion.kind.FUNCTION)
            local listed = completion.list({ item })
            assert(#listed.items == 1)

            local types = require("hexal.lsp.types")
            local edit = require("hexal.lsp.edit")
            local textEdit = edit.text_edit(
                types.range(types.position(0, 0), types.position(0, 1)), "H")
            assert(textEdit.newText == "H")
            local virtual = require("hexal.lsp.documents").create_virtual(
                "untitled:test", "hello", "plaintext")
            assert(virtual and virtual.text == "hello")
            local ok, applyError = edit.apply_workspace_edit({
                changes = { ["untitled:test"] = { textEdit } },
            })
            assert(ok, applyError)
            assert(require("hexal.lsp.documents").get("untitled:test").text == "Hello")

            local decoded = json.decode("[null,1]")
            assert(decoded[1] == json.null and decoded[2] == 1)
            local ran = false
            require("hexal.async").run(function() ran = true end)
            assert(ran)

            local position = require("hexal.lsp.position")
            local live = require("hexal.lsp.documents").get("file:///test.tst")
            assert(position.position_to_offset(live, { line = 0, character = 5 }) == 5)
            assert(position.offset_to_position(live, 5).character == 5)

            local idx = require("hexal.index").create({})
            idx:add_file("file:///symbols", {
                { name = "greet", kind = 12, range = types.range(
                    types.position(0, 0), types.position(0, 5)) },
            })
            local found = idx:query_symbols("greet", {})
            assert(#found == 1 and found[1].name == "greet")

            local caps = server:advertised_capabilities()
            assert(caps.hoverProvider == true)

            local outgoingResult = nil
            local handle = server:send_request(
                "workspace/configuration",
                { items = { { section = "test" } } },
                function(result, err)
                    outgoingResult = result
                end)
            assert(handle:id() > 0 and handle:is_pending())
            client:tick()
            assert(outgoingResult ~= nil)
            assert(not handle:is_pending())

            hexal.ui.message("LUA_LSP_OK")
            """;

        File.WriteAllText(
            System.IO.Path.Combine(plugin, "init.lua"),
            script,
            new UTF8Encoding(false)
        );

        var host = new FakeHost();
        using var manager = new LuaPluginManager(host, plugins);
        manager.LoadAll();

        if (manager.Errors.Count > 0)
        {
            Console.Error.WriteLine(string.Join("\n", manager.Errors));
        }
        CheckEqual(manager.Errors.Count, 0, "lua lsp errors", "0");
        CheckEqual(manager.PluginCount, 1, "lua lsp plugin count", "1");
        Check(host.Messages.Contains("LUA_LSP_OK"), "lua lsp script completed");
        manager.Tick();

        string sourcePath = directory.Path("sample.tst");
        Check(WriteBytes(sourcePath, "hello"u8), "write lua lsp document");
        LuaLspCoordinator coordinator = manager.Coordinators[0];
        LuaLspServer server = coordinator.Servers[0];
        using var lsp = new LspManager();
        var transport = new LuaLspTransport(server);
        Check(
            lsp.TryStart(
                new LspServerConfig
                {
                    Name = server.Name,
                    LanguageId = "test",
                    Transport = transport,
                },
                sourcePath,
                "hello",
                directory.Path("."),
                out string lspError
            ),
            $"start in-process lua lsp ({lspError})"
        );
        lsp.Pump();
        Check(lsp.IsInitialized, "in-process lua lsp initialized");
        Check(lsp.RequestHover(sourcePath, 0, 1), "request lua lsp hover");
        lsp.Pump();
        bool hoverSeen = false;
        while (lsp.TryDequeueEvent(out LspEvent lspEvent))
        {
            if (
                lspEvent.Kind == LspEventKind.Hover
                && lspEvent.Text.Contains("hover-doc", StringComparison.Ordinal)
            )
            {
                hoverSeen = true;
            }
        }
        Check(hoverSeen, "in-process lua lsp hover event");

        Check(lsp.RequestDefinition(sourcePath, 0, 1), "request lua lsp definition");
        lsp.Pump();
        bool definitionSeen = false;
        while (lsp.TryDequeueEvent(out LspEvent lspEvent))
        {
            if (
                lspEvent.Kind == LspEventKind.Definition
                && lspEvent.Locations.Count == 1
                && lspEvent.Locations[0].Uri == "file:///test.tst"
                && lspEvent.Locations[0].Line == 0
                && lspEvent.Locations[0].Character == 2
            )
            {
                definitionSeen = true;
            }
        }
        Check(definitionSeen, "in-process lua lsp definition event");
    }

    private sealed class FakeHost : IEditorApi
    {
        public List<string> Messages { get; } = new();

        public string CurrentBufferId => "[No Name]";

        public string? WorkspaceRoot { get; set; }

        public int PanelId { get; private set; }

        public IReadOnlyList<PluginPanelLine> PanelLines { get; private set; } =
            Array.Empty<PluginPanelLine>();

        public Func<string, long, bool>? PanelKeyHandler { get; private set; }

        public int OpenPluginPanel(
            PluginPanelOptions options,
            Func<string, long, bool>? onKey,
            string pluginId
        )
        {
            PanelId = 1;
            PanelLines = options.Lines;
            PanelKeyHandler = onKey;
            return PanelId;
        }

        public void SetPluginPanelLines(int panelId, IReadOnlyList<PluginPanelLine> lines) =>
            PanelLines = lines;

        public void SetPluginPanelTitle(int panelId, string title) { }

        public void SetPluginPanelFooter(int panelId, string footer) { }

        public void SetPluginPanelSelected(int panelId, long selected) { }

        public void SetPluginPanelSelectable(int panelId, bool selectable) { }

        public void ClosePluginPanel(int panelId) { }

        public void ClosePluginPanels(string pluginId) { }

        public string GetText() => string.Empty;

        public string? GetLine(long line, out string error)
        {
            error = string.Empty;
            return null;
        }

        public PluginCursor GetCursor() => new(0, 0);

        public bool SetCursor(long line, long column, out string error)
        {
            error = string.Empty;
            return true;
        }

        public PluginSelection? GetSelection() => null;

        public bool InsertText(string text, out string error)
        {
            error = string.Empty;
            return true;
        }

        public bool ReplaceRange(object? start, object? finish, string text, out string error)
        {
            error = string.Empty;
            return true;
        }

        public void ShowMessage(string text, bool warning) => Messages.Add(text);

        public void SetPluginStatus(string key, string? text) { }

        public void RequestPrompt(string label, Action<string?> callback) => callback(null);

        public void RequestSelect(
            string title,
            IReadOnlyList<string> items,
            Action<int?> callback
        ) => callback(null);

        public bool StartLsp(LspServerConfig config, out string error)
        {
            error = string.Empty;
            return true;
        }

        public void StopLsp(string? name) { }

        public bool ExecuteCommand(string name, IReadOnlyList<string> args, out string error)
        {
            error = string.Empty;
            return false;
        }

        public void ReportPluginError(string pluginId, string message) =>
            Messages.Add($"error: {pluginId}: {message}");
    }

    private static void TestHexalPaths()
    {
        using var binary = new TempDir();
        using var home = new TempDir();

        string fallback = HexalPaths.ResolveDataDirectory(
            binary.Path("."),
            home.Path("."),
            "grammars"
        );
        CheckEqual(
            fallback,
            System.IO.Path.Combine(home.Path("."), ".hexal", "data", "grammars"),
            "fallback grammar path",
            "home data path"
        );

        string portableDirectory = System.IO.Path.Combine(binary.Path("."), "data", "grammars");
        Directory.CreateDirectory(portableDirectory);
        string portable = HexalPaths.ResolveDataDirectory(
            binary.Path("."),
            home.Path("."),
            "grammars"
        );
        CheckEqual(portable, portableDirectory, "portable grammar path", "binary data path");

        string themeFallback = HexalPaths.ResolveDataDirectory(
            binary.Path("."),
            home.Path("."),
            "themes"
        );
        CheckEqual(
            themeFallback,
            System.IO.Path.Combine(home.Path("."), ".hexal", "data", "themes"),
            "fallback theme path",
            "home data path"
        );
    }

    private static long ExpectedLineStart(byte[] text, long line)
    {
        if (line == 0)
        {
            return 0;
        }

        long seen = 0;
        for (int i = 0; i < text.Length; ++i)
        {
            if (text[i] == (byte)'\n')
            {
                ++seen;
                if (seen == line)
                {
                    return i + 1;
                }
            }
        }
        return text.Length;
    }

    private static long ExpectedLineOfOffset(byte[] text, long offset)
    {
        long line = 0;
        long limit = Math.Min(offset, text.Length);
        for (int i = 0; i < limit; ++i)
        {
            if (text[i] == (byte)'\n')
            {
                ++line;
            }
        }
        return line;
    }

    private static bool WriteBytes(string path, ReadOnlySpan<byte> data)
    {
        try
        {
            using var stream = new FileStream(
                path,
                FileMode.Create,
                FileAccess.Write,
                FileShare.None
            );
            stream.Write(data);
            return true;
        }
        catch (IOException)
        {
            return false;
        }
    }

    private static byte[] ReadBytes(string path)
    {
        try
        {
            return File.ReadAllBytes(path);
        }
        catch (IOException)
        {
            return Array.Empty<byte>();
        }
    }

    private sealed class TempDir : IDisposable
    {
        private readonly string _directory;

        public TempDir()
        {
            _directory = System.IO.Path.Combine(
                System.IO.Path.GetTempPath(),
                $"hexal-tests-{Guid.NewGuid():N}"
            );
            Directory.CreateDirectory(_directory);
        }

        public string Path(string name) => System.IO.Path.Combine(_directory, name);

        public void Dispose()
        {
            try
            {
                Directory.Delete(_directory, recursive: true);
            }
            catch (IOException)
            {
                // Best effort cleanup.
            }
        }
    }
}

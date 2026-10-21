using System.Runtime.CompilerServices;
using System.Text;
using Hexal.Core;
using Hexal.Languages;
using Hexal.Scripting;
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
        TestUndoRedo();
        TestSaveFailureLeavesOriginal();
        TestFilesystemReadWrite();
        TestFileSearch();
        TestLanguageGrammars();
        TestPluginConfigStore();

        Console.Out.WriteLine($"{s_checks - s_failures} of {s_checks} checks passed");
        if (s_failures != 0)
        {
            Console.Error.WriteLine($"{s_failures} check(s) failed");
            return 1;
        }
        return 0;
    }

    private static void Check(bool condition, string expression,
        [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        ++s_checks;
        if (!condition)
        {
            ++s_failures;
            Console.Error.WriteLine($"{file}:{line}: CHECK failed: {expression}");
        }
    }

    private static void CheckEqual<T>(T actual, T expected, string actualText,
        string expectedText, [CallerFilePath] string file = "", [CallerLineNumber] int line = 0)
    {
        ++s_checks;
        if (!EqualityComparer<T>.Default.Equals(actual, expected))
        {
            ++s_failures;
            Console.Error.WriteLine(
                $"{file}:{line}: CHECK_EQ failed: {actualText} == {expectedText}\n" +
                $"  actual:   {actual}\n  expected: {expected}\n");
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

        string[] insertions = { "x", "YZ", "\n", "alpha\n", "\t", " ", "\r\n", "123", "longer text here" };
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
                Check(table.ToArray().AsSpan().SequenceEqual(reference.ToArray()),
                    "randomized table == reference");
            }
        }

        byte[] final = reference.ToArray();
        Check(table.ToArray().AsSpan().SequenceEqual(final), "final table == reference");
        CheckEqual(table.NewlineCount, (long)final.Count(b => b == '\n'),
            "newline count", "reference newline count");

        long lineCount = table.NewlineCount + 1;
        for (long line = 0; line < lineCount; line += 17)
        {
            CheckEqual(table.LineStart(line), ExpectedLineStart(final, line),
                $"lineStart({line})", "expectedLineStart");
        }
        CheckEqual(table.LineStart(lineCount), (long)final.Length,
            "lineStart(lineCount)", "reference length");

        for (int i = 0; i < 200; ++i)
        {
            long offset = random.Next(final.Length + 1);
            CheckEqual(table.LineOfOffset(offset), ExpectedLineOfOffset(final, offset),
                $"lineOfOffset({offset})", "expectedLineOfOffset");
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
        Check(table.Range(6, payload.Length).AsSpan().SequenceEqual(payload),
            "large inserted range matches payload");
        CheckEqual(table.NewlineCount, 1L + payload.Count(b => b == '\n'),
            "large insert newline count", "expected");

        long middleLine = table.LineOfOffset(1_000_000);
        long start = table.LineStart(middleLine);
        Check(start <= 1_000_000 && (start == 1_000_000 || table.ByteAt(start - 1) == '\n'),
            "offset maps to a valid line start");
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
        CheckEqual(Encoding.UTF8.GetString(stream.ToArray()), "Ahello\n",
            "snapshot text", "Ahello");
        CheckEqual(Text(table.ToArray()), "BAhello\ntail",
            "live text after snapshot", "BAhello\ntail");

        // Appending to the add buffer after a snapshot must not change the
        // snapshot, even when the append crosses a storage chunk boundary.
        PieceTable.Snapshot beforeLargeAppend = table.CreateSnapshot();
        byte[] payload = new byte[1_500_000];
        Array.Fill(payload, (byte)'z');
        table.Insert(table.Size, payload);

        using var largeStream = new MemoryStream();
        beforeLargeAppend.WriteTo(largeStream);
        CheckEqual(largeStream.Length, 12L, "snapshot length after large append", "12");
        CheckEqual(Encoding.UTF8.GetString(largeStream.ToArray()), "BAhello\ntail",
            "snapshot unaffected by later append", "BAhello\ntail");

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
            directory.Path("saved-snapshot.txt"), out error);
        Check(docSnapshot is not null, "create document save snapshot");
        if (docSnapshot is null)
        {
            return;
        }

        document.Insert(0, "y"u8);
        Check(docSnapshot.WriteTo(out error), "write document snapshot");
        CheckEqual(Text(ReadBytes(directory.Path("saved-snapshot.txt"))), "xbase\n",
            "document snapshot contents", "xbase");

        document.MarkSaved(directory.Path("saved-snapshot.txt"), docSnapshot.Pose,
            docSnapshot.Revision);
        Check(document.IsDirty, "edits after snapshot keep document dirty");
        CheckEqual(document.Path ?? "", directory.Path("saved-snapshot.txt"),
            "mark saved updates path", "snapshot path");

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
            branchDirectory.Path("branch-saved.txt"), out error);
        Check(branchSnapshot is not null, "create branch snapshot");
        if (branchSnapshot is null)
        {
            return;
        }

        Check(branch.Undo() is not null, "undo branch snapshot edit");
        branch.Insert(0, "y"u8);
        branch.MarkSaved(branchDirectory.Path("branch-saved.txt"),
            branchSnapshot.Pose, branchSnapshot.Revision);
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
        CheckEqual(UnicodeUtil.ByteOffsetForVisualColumn(line, 4, 4), 2,
            "byte offset for column 4", "2");
        CheckEqual(UnicodeUtil.ByteOffsetForVisualColumn(line, 5, 4), 2,
            "byte offset for column 5", "2");
        CheckEqual(UnicodeUtil.ByteOffsetForVisualColumn(line, 6, 4), line.Length,
            "byte offset for column 6", "length");

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
        CheckEqual(Text(ReadBytes(crlfPath)), "alpha\r\n\r\nbeta\r\ngamma", "saved CRLF", "expected");
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
        Check(WriteBytes(bomPath, new byte[] { 0xEF, 0xBB, 0xBF, (byte)'h', (byte)'i', (byte)'\n' }),
            "write BOM file");
        Document? bom = Document.Open(bomPath, out error);
        Check(bom is not null, "open BOM document");
        if (bom is not null)
        {
            Check(bom.HasBom, "BOM detected");
            CheckEqual(Text(bom.Text()), "hi\n", "BOM stripped in memory", "hi");
            Check(bom.Save(out error), "save BOM document");
            Check(ReadBytes(bomPath).AsSpan().SequenceEqual(
                    new byte[] { 0xEF, 0xBB, 0xBF, (byte)'h', (byte)'i', (byte)'\n' }),
                "BOM preserved on save");
        }
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
        CheckEqual(Text(document.Text()), "changed original\n", "in-memory text retained", "changed");
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

        Check(FileSystemUtil.AtomicWrite(path, stream => stream.Write("new"u8), out string error),
            "atomic write");
        CheckEqual(Text(ReadBytes(path)), "new", "atomic contents", "new");

        string missing = System.IO.Path.Combine(directory.Path("nope"), "x.txt");
        Check(!FileSystemUtil.AtomicWrite(missing, stream => stream.Write("x"u8), out error),
            "atomic write to missing directory fails");

        Check(FileSystemUtil.ReadFile(path, out byte[] contents, out error), "read file");
        CheckEqual(Text(contents), "new", "read contents", "new");
    }

    private static void TestFileSearch()
    {
        using var directory = new TempDir();
        string source = directory.Path("src");
        string ignored = directory.Path("build");
        Directory.CreateDirectory(source);
        Directory.CreateDirectory(ignored);

        Check(WriteBytes(directory.Path("root.txt"), "first line\nsecond line\n"u8),
            "write search root file");
        Check(WriteBytes(System.IO.Path.Combine(source, "code.cs"),
            "alpha\n  beta needle gamma\n"u8), "write search nested file");
        Check(WriteBytes(System.IO.Path.Combine(ignored, "skip.cs"),
            "needle in ignored directory\n"u8), "write ignored file");

        List<string> files = FileSearch.EnumerateFiles(directory.Path("."),
            FileSearch.DefaultMaxFiles, CancellationToken.None);

        Check(files.Contains(System.IO.Path.Combine("src", "code.cs")),
            "nested file is enumerated");
        Check(files.Contains("root.txt"), "root file is enumerated");
        Check(!files.Any(path => path.StartsWith("build", StringComparison.Ordinal)),
            "ignored directory is skipped");

        List<FileSearchMatch> matches = FileSearch.Search(directory.Path("."), files,
            "needle", FileSearch.DefaultMaxResults, CancellationToken.None);

        CheckEqual(matches.Count, 1, "matches.Count", "1");
        CheckEqual(matches[0].Path, System.IO.Path.Combine("src", "code.cs"),
            "match.Path", "nested code.cs");
        CheckEqual(matches[0].Line, 1L, "match.Line", "1");
        CheckEqual(matches[0].Column, 7L, "match.Column", "7");
        Check(matches[0].Preview.Contains("beta needle", StringComparison.Ordinal),
            "match preview");
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
                "comments": {
                    "line": "//",
                    "blockStart": "/*",
                    "blockEnd": "*/"
                },
                "strings": {
                    "quoteCharacters": ["\""],
                    "escapeCharacter": "\\"
                }
            }
            """;

        Check(LanguageDefinition.TryParse(grammarJson, "test.hexdef",
                out LanguageDefinition? language, out string parseError),
            "parse grammar");
        CheckEqual(parseError, string.Empty, "parse error", "empty");
        Check(language is not null, "grammar parsed");
        LanguageDefinition definition = language!;
        CheckEqual(definition.Id, "test", "grammar id", "test");
        CheckEqual(definition.Extensions[0], ".tst", "grammar extension", ".tst");

        using var directory = new TempDir();
        string grammarPath = directory.Path("test.hexdef");
        Check(WriteBytes(grammarPath, Encoding.UTF8.GetBytes(grammarJson)),
            "write grammar file");

        var registry = new LanguageRegistry();
        registry.LoadDirectory(directory.Path("."));
        CheckEqual(registry.Errors.Count, 0, "registry errors", "0");
        Check(ReferenceEquals(registry.Detect("sample.tst"), registry.Find("test")),
            "extension detection");

        string sourcePath = directory.Path("sample.tst");
        const string source = "fn main() {\n  let x = 42;\n  // comment\n  /* block\n     still */\n  int y = true;\n}\n";
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
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Keyword),
            "keyword token");
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.ControlStatement) == false,
            "no control token on line 0");

        highlighter.GetLineTokens(1, document.LineText(1), tokens);
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Number),
            "number token");
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Keyword),
            "line 1 keyword token");

        highlighter.GetLineTokens(2, document.LineText(2), tokens);
        Check(tokens.Count == 1 && tokens[0].Kind == SyntaxTokenKind.Comment,
            "line comment token");

        highlighter.GetLineTokens(3, document.LineText(3), tokens);
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Comment),
            "block comment start token");

        highlighter.GetLineTokens(4, document.LineText(4), tokens);
        Check(tokens.Count == 1 && tokens[0].Kind == SyntaxTokenKind.Comment,
            "continued block comment token");

        highlighter.GetLineTokens(5, document.LineText(5), tokens);
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Type), "type token");
        Check(tokens.Any(token => token.Kind == SyntaxTokenKind.Constant),
            "constant token");
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
        CheckEqual(Convert.ToInt64(loaded.Get("answer")), 42L,
            "config answer", "42");
        CheckEqual(loaded.Get("nested.name") as string, "hexal",
            "nested config value", "hexal");
        CheckEqual(loaded.Get("missing", "fallback") as string, "fallback",
            "config fallback", "fallback");
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
            using var stream = new FileStream(path, FileMode.Create, FileAccess.Write,
                FileShare.None);
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
            _directory = System.IO.Path.Combine(System.IO.Path.GetTempPath(),
                $"hexal-tests-{Guid.NewGuid():N}");
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

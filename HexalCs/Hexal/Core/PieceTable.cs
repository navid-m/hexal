namespace Hexal.Core;

/// <summary>
/// A piece table over immutable original chunks and an append-only add buffer.
/// The piece sequence is stored in an implicit treap whose nodes live in one
/// array and are recycled through a free list. Every node is augmented with the
/// byte and newline counts of its subtree, giving O(log p) edits, offset-to-line
/// and line-to-offset queries.
///
/// Text is stored in chunks so a document is not limited to one contiguous
/// managed array. Pieces are split at roughly 8 KiB newline boundaries, which
/// bounds newline scans and avoids copying text when a piece splits.
/// </summary>
public sealed class PieceTable
{
    public const int MaxPieceSize = 8192;

    private const int ChunkSize = 1 << 20;

    private struct Chunk
    {
        public byte[] Data;
        public int Length;

        public Chunk(byte[] data, int length)
        {
            Data = data;
            Length = length;
        }
    }

    private struct Node
    {
        public int Left;
        public int Right;
        public int ChunkIndex;
        public int Offset;
        public int Length;
        public int PieceNewlines;
        public long Bytes;
        public int Newlines;
        public ulong Priority;
        public bool FromAdd;
    }

    private readonly List<Chunk> _original = new();
    private readonly List<Chunk> _added = new();

    private Node[] _nodes = new Node[256];
    private int _nodeCount;
    private int _freeHead = -1;
    private int _root = -1;
    private ulong _randomState = 0x9E3779B97F4A7C15UL;

    public string DetectedEol { get; private set; } = "\n";

    public long Size => _root < 0 ? 0 : _nodes[_root].Bytes;

    public long NewlineCount => _root < 0 ? 0 : _nodes[_root].Newlines;

    public void SetOriginal(Stream stream)
    {
        Reset();

        bool foundNewline = false;
        int previousByte = -1;

        while (true)
        {
            byte[] data = new byte[ChunkSize];
            int read = stream.Read(data, 0, data.Length);
            if (read <= 0)
            {
                break;
            }

            if (!foundNewline)
            {
                foundNewline = DetectEol(data.AsSpan(0, read), previousByte);
            }

            AddOriginalChunk(data, read);
            previousByte = data[read - 1];
        }

        if (!foundNewline)
        {
            DetectedEol = "\n";
        }
    }

    public void SetOriginal(ReadOnlySpan<byte> original)
    {
        Reset();

        bool foundNewline = false;
        int previousByte = -1;
        int position = 0;

        while (position < original.Length)
        {
            int take = Math.Min(ChunkSize, original.Length - position);
            byte[] data = new byte[take];
            original.Slice(position, take).CopyTo(data);

            if (!foundNewline)
            {
                foundNewline = DetectEol(data, previousByte);
            }

            AddOriginalChunk(data, take);
            previousByte = data[take - 1];
            position += take;
        }

        if (!foundNewline)
        {
            DetectedEol = "\n";
        }
    }

    public void Insert(long offset, ReadOnlySpan<byte> text)
    {
        if (text.IsEmpty)
        {
            return;
        }

        offset = Math.Clamp(offset, 0, Size);
        int insertedRoot = AppendAdded(text);

        Split(_root, offset, out int left, out int right);
        _root = Merge(Merge(left, insertedRoot), right);
    }

    public void Erase(long offset, long length)
    {
        if (length <= 0 || offset >= Size)
        {
            return;
        }

        length = Math.Min(length, Size - offset);

        Split(_root, offset, out int left, out int rest);
        Split(rest, length, out int removed, out int right);
        FreeTree(removed);
        _root = Merge(left, right);
    }

    public byte[] ToArray() => Range(0, Size);

    public byte[] Range(long offset, long length)
    {
        long total = Size;
        if (offset >= total || length <= 0)
        {
            return Array.Empty<byte>();
        }

        length = Math.Min(length, total - offset);
        if (length > int.MaxValue)
        {
            throw new ArgumentOutOfRangeException(nameof(length),
                "a single managed range cannot exceed int.MaxValue bytes");
        }

        byte[] result = new byte[(int)length];
        int written = 0;
        CollectRange(_root, 0, offset, offset + length, result, ref written);
        return result;
    }

    /// <summary>
    /// Copies a range into a reusable buffer. Used by the renderer for visible
    /// lines so a frame does not allocate one string per line.
    /// </summary>
    public int CopyRange(long offset, long length, ref byte[] buffer)
    {
        long total = Size;
        if (offset >= total || length <= 0)
        {
            return 0;
        }

        length = Math.Min(length, total - offset);
        if (length > int.MaxValue)
        {
            throw new ArgumentOutOfRangeException(nameof(length),
                "a single managed range cannot exceed int.MaxValue bytes");
        }

        int needed = (int)length;
        if (buffer.Length < needed)
        {
            buffer = new byte[Math.Max(needed, 256)];
        }

        int written = 0;
        CollectRange(_root, 0, offset, offset + length, buffer, ref written);
        return written;
    }

    public byte ByteAt(long offset)
    {
        if (offset < 0 || offset >= Size)
        {
            return 0;
        }

        long baseOffset = 0;
        int node = _root;

        while (node >= 0)
        {
            long leftBytes = BytesOf(_nodes[node].Left);
            long nodeStart = baseOffset + leftBytes;

            if (offset < nodeStart)
            {
                node = _nodes[node].Left;
                continue;
            }

            if (offset < nodeStart + _nodes[node].Length)
            {
                Node current = _nodes[node];
                Chunk chunk = SourceChunk(current);
                return chunk.Data[current.Offset + (int)(offset - nodeStart)];
            }

            baseOffset = nodeStart + _nodes[node].Length;
            node = _nodes[node].Right;
        }

        return 0;
    }

    public void WriteTo(Stream output) => WriteNode(_root, output);

    /// <summary>
    /// Captures the current piece sequence and the immutable storage chunks it
    /// references. The returned snapshot can be written from another thread
    /// while the live table continues to be edited: existing bytes are never
    /// modified, only appended after the ranges in the snapshot.
    /// </summary>
    public Snapshot CreateSnapshot()
    {
        var pieces = new List<PieceRef>();
        CollectPieceRefs(_root, pieces);

        var original = new byte[_original.Count][];
        for (int i = 0; i < _original.Count; ++i)
        {
            original[i] = _original[i].Data;
        }

        var added = new byte[_added.Count][];
        for (int i = 0; i < _added.Count; ++i)
        {
            added[i] = _added[i].Data;
        }

        return new Snapshot(original, added, pieces.ToArray());
    }

    public readonly struct PieceRef
    {
        public readonly bool FromAdd;
        public readonly int ChunkIndex;
        public readonly int Offset;
        public readonly int Length;

        public PieceRef(bool fromAdd, int chunkIndex, int offset, int length)
        {
            FromAdd = fromAdd;
            ChunkIndex = chunkIndex;
            Offset = offset;
            Length = length;
        }
    }

    public sealed class Snapshot
    {
        private readonly byte[][] _original;
        private readonly byte[][] _added;
        private readonly PieceRef[] _pieces;

        internal Snapshot(byte[][] original, byte[][] added, PieceRef[] pieces)
        {
            _original = original;
            _added = added;
            _pieces = pieces;
        }

        public void WriteTo(Stream output)
        {
            foreach (PieceRef piece in _pieces)
            {
                byte[] source = piece.FromAdd
                    ? _added[piece.ChunkIndex]
                    : _original[piece.ChunkIndex];
                output.Write(source.AsSpan(piece.Offset, piece.Length));
            }
        }
    }

    public long LineStart(long line)
    {
        if (line <= 0 || _root < 0)
        {
            return 0;
        }

        long remaining = line;
        long baseOffset = 0;
        int node = _root;

        while (node >= 0)
        {
            long leftNewlines = NewlinesOf(_nodes[node].Left);

            if (remaining <= leftNewlines)
            {
                node = _nodes[node].Left;
                continue;
            }
            remaining -= leftNewlines;

            int pieceNewlines = _nodes[node].PieceNewlines;
            if (remaining <= pieceNewlines)
            {
                int index = NewlineOffsetInPiece(node, (int)remaining);
                return baseOffset + BytesOf(_nodes[node].Left) + index + 1;
            }
            remaining -= pieceNewlines;

            baseOffset += BytesOf(_nodes[node].Left) + _nodes[node].Length;
            node = _nodes[node].Right;
        }

        return Size;
    }

    public long LineOfOffset(long offset)
    {
        if (offset >= Size)
        {
            return NewlineCount;
        }

        long line = 0;
        long baseOffset = 0;
        int node = _root;

        while (node >= 0)
        {
            long leftBytes = BytesOf(_nodes[node].Left);
            long nodeStart = baseOffset + leftBytes;

            if (offset < nodeStart)
            {
                node = _nodes[node].Left;
                continue;
            }

            if (offset < nodeStart + _nodes[node].Length)
            {
                int local = (int)(offset - nodeStart);
                Node current = _nodes[node];
                Chunk chunk = SourceChunk(current);
                return line + NewlinesOf(current.Left) +
                       CountNewlines(chunk.Data, current.Offset, local);
            }

            line += NewlinesOf(_nodes[node].Left) + _nodes[node].PieceNewlines;
            baseOffset = nodeStart + _nodes[node].Length;
            node = _nodes[node].Right;
        }

        return line;
    }

    private void Reset()
    {
        _original.Clear();
        _added.Clear();
        _root = -1;
        _nodeCount = 0;
        _freeHead = -1;
        DetectedEol = "\n";
    }

    private bool DetectEol(ReadOnlySpan<byte> data, int previousByte)
    {
        for (int i = 0; i < data.Length; ++i)
        {
            if (data[i] != (byte)'\n')
            {
                continue;
            }

            int before = i > 0 ? data[i - 1] : previousByte;
            DetectedEol = before == '\r' ? "\r\n" : "\n";
            return true;
        }

        return false;
    }

    private void AddOriginalChunk(byte[] data, int length)
    {
        if (length <= 0)
        {
            return;
        }

        _original.Add(new Chunk(data, length));
        int chunkIndex = _original.Count - 1;

        int position = 0;
        while (position < length)
        {
            int end = ChooseChunkEnd(data, position, length);
            _root = Merge(_root, MakeNode(false, chunkIndex, position, end - position));
            position = end;
        }
    }

    private int AppendAdded(ReadOnlySpan<byte> text)
    {
        int insertedRoot = -1;

        while (!text.IsEmpty)
        {
            if (_added.Count == 0 || _added[^1].Length == ChunkSize)
            {
                _added.Add(new Chunk(new byte[ChunkSize], 0));
            }

            int chunkIndex = _added.Count - 1;
            Chunk chunk = _added[chunkIndex];
            int take = Math.Min(ChunkSize - chunk.Length, text.Length);

            text[..take].CopyTo(chunk.Data.AsSpan(chunk.Length));
            int start = chunk.Length;
            int end = start + take;
            chunk.Length = end;
            _added[chunkIndex] = chunk;

            int position = start;
            while (position < end)
            {
                int pieceEnd = ChooseChunkEnd(chunk.Data, position, end);
                insertedRoot = Merge(insertedRoot,
                    MakeNode(true, chunkIndex, position, pieceEnd - position));
                position = pieceEnd;
            }

            text = text[take..];
        }

        return insertedRoot;
    }

    private static int ChooseChunkEnd(byte[] source, int start, int limit)
    {
        if (start >= limit)
        {
            return limit;
        }

        int maxEnd = Math.Min(limit, start + MaxPieceSize);
        if (maxEnd < limit)
        {
            int newline = source.AsSpan(start, maxEnd - start).LastIndexOf((byte)'\n');
            if (newline >= 0)
            {
                return start + newline + 1;
            }
        }

        return maxEnd;
    }

    private Chunk SourceChunk(in Node node)
        => node.FromAdd ? _added[node.ChunkIndex] : _original[node.ChunkIndex];

    private int MakeNode(bool fromAdd, int chunkIndex, int offset, int length)
    {
        int index = AllocNode();
        Node node = new()
        {
            Left = -1,
            Right = -1,
            FromAdd = fromAdd,
            ChunkIndex = chunkIndex,
            Offset = offset,
            Length = length,
            Priority = NextPriority()
        };

        Chunk chunk = fromAdd ? _added[chunkIndex] : _original[chunkIndex];
        node.PieceNewlines = CountNewlines(chunk.Data, offset, length);
        _nodes[index] = node;
        Update(index);
        return index;
    }

    private int AllocNode()
    {
        int index;
        if (_freeHead >= 0)
        {
            index = _freeHead;
            _freeHead = _nodes[index].Left;
        }
        else
        {
            if (_nodeCount == _nodes.Length)
            {
                Array.Resize(ref _nodes, _nodes.Length * 2);
            }
            index = _nodeCount++;
        }

        _nodes[index] = default;
        return index;
    }

    private void FreeNode(int index)
    {
        _nodes[index].Left = _freeHead;
        _freeHead = index;
    }

    private void FreeTree(int root)
    {
        if (root < 0)
        {
            return;
        }

        int left = _nodes[root].Left;
        int right = _nodes[root].Right;
        FreeTree(left);
        FreeTree(right);
        FreeNode(root);
    }

    private long BytesOf(int node) => node < 0 ? 0 : _nodes[node].Bytes;

    private int NewlinesOf(int node) => node < 0 ? 0 : _nodes[node].Newlines;

    private void Update(int index)
    {
        Node node = _nodes[index];
        long leftBytes = BytesOf(node.Left);
        long rightBytes = BytesOf(node.Right);
        int leftNewlines = NewlinesOf(node.Left);
        int rightNewlines = NewlinesOf(node.Right);

        node.Bytes = leftBytes + node.Length + rightBytes;
        node.Newlines = leftNewlines + node.PieceNewlines + rightNewlines;
        _nodes[index] = node;
    }

    private int Merge(int left, int right)
    {
        if (left < 0)
        {
            return right;
        }
        if (right < 0)
        {
            return left;
        }

        if (_nodes[left].Priority >= _nodes[right].Priority)
        {
            int oldRight = _nodes[left].Right;
            int merged = Merge(oldRight, right);
            _nodes[left].Right = merged;
            Update(left);
            return left;
        }
        else
        {
            int oldLeft = _nodes[right].Left;
            int merged = Merge(left, oldLeft);
            _nodes[right].Left = merged;
            Update(right);
            return right;
        }
    }

    private void Split(int tree, long offset, out int left, out int right)
    {
        if (tree < 0)
        {
            left = -1;
            right = -1;
            return;
        }

        long leftBytes = BytesOf(_nodes[tree].Left);
        int nodeLength = _nodes[tree].Length;

        if (offset < leftBytes)
        {
            int oldLeft = _nodes[tree].Left;
            Split(oldLeft, offset, out int lhs, out int rhs);
            _nodes[tree].Left = rhs;
            Update(tree);
            left = lhs;
            right = tree;
            return;
        }

        if (offset > leftBytes + nodeLength)
        {
            int oldRight = _nodes[tree].Right;
            Split(oldRight, offset - leftBytes - nodeLength, out int lhs, out int rhs);
            _nodes[tree].Right = lhs;
            Update(tree);
            left = tree;
            right = rhs;
            return;
        }

        if (offset == leftBytes)
        {
            int oldLeft = _nodes[tree].Left;
            _nodes[tree].Left = -1;
            Update(tree);
            left = oldLeft;
            right = tree;
            return;
        }

        if (offset == leftBytes + nodeLength)
        {
            int oldRight = _nodes[tree].Right;
            _nodes[tree].Right = -1;
            Update(tree);
            left = tree;
            right = oldRight;
            return;
        }

        // The split point is inside this piece. Both halves keep the original
        // priority so the heap invariant relative to the existing children is
        // preserved. No text is copied.
        int localOffset = (int)(offset - leftBytes);
        Node original = _nodes[tree];
        Chunk source = SourceChunk(original);
        int leftNewlines = CountNewlines(source.Data, original.Offset, localOffset);

        int leftHalf = AllocNode();
        _nodes[leftHalf] = new Node
        {
            Left = original.Left,
            Right = -1,
            FromAdd = original.FromAdd,
            ChunkIndex = original.ChunkIndex,
            Offset = original.Offset,
            Length = localOffset,
            PieceNewlines = leftNewlines,
            Priority = original.Priority
        };
        Update(leftHalf);

        int rightHalf = AllocNode();
        _nodes[rightHalf] = new Node
        {
            Left = -1,
            Right = original.Right,
            FromAdd = original.FromAdd,
            ChunkIndex = original.ChunkIndex,
            Offset = original.Offset + localOffset,
            Length = original.Length - localOffset,
            PieceNewlines = original.PieceNewlines - leftNewlines,
            Priority = original.Priority
        };
        Update(rightHalf);

        FreeNode(tree);
        left = leftHalf;
        right = rightHalf;
    }

    private void CollectRange(int node, long baseOffset, long begin, long end,
                              Span<byte> destination, ref int written)
    {
        if (node < 0 || begin >= end)
        {
            return;
        }

        Node current = _nodes[node];
        long leftBytes = BytesOf(current.Left);
        long nodeStart = baseOffset + leftBytes;
        long nodeEnd = nodeStart + current.Length;

        if (begin < nodeStart)
        {
            CollectRange(current.Left, baseOffset, begin, Math.Min(end, nodeStart),
                destination, ref written);
        }

        if (begin < nodeEnd && end > nodeStart)
        {
            int from = (int)(Math.Max(begin, nodeStart) - nodeStart);
            int to = (int)(Math.Min(end, nodeEnd) - nodeStart);
            Chunk chunk = SourceChunk(current);
            chunk.Data.AsSpan(current.Offset + from, to - from)
                .CopyTo(destination[written..]);
            written += to - from;
        }

        if (end > nodeEnd)
        {
            CollectRange(current.Right, nodeEnd, Math.Max(begin, nodeEnd), end,
                destination, ref written);
        }
    }

    private void CollectPieceRefs(int node, List<PieceRef> pieces)
    {
        if (node < 0)
        {
            return;
        }

        Node current = _nodes[node];
        CollectPieceRefs(current.Left, pieces);
        pieces.Add(new PieceRef(current.FromAdd, current.ChunkIndex,
            current.Offset, current.Length));
        CollectPieceRefs(current.Right, pieces);
    }

    private void WriteNode(int node, Stream output)
    {
        if (node < 0)
        {
            return;
        }

        WriteNode(_nodes[node].Left, output);

        Node current = _nodes[node];
        Chunk chunk = SourceChunk(current);
        output.Write(chunk.Data.AsSpan(current.Offset, current.Length));

        WriteNode(current.Right, output);
    }

    private int NewlineOffsetInPiece(int node, int nth)
    {
        Node current = _nodes[node];
        Chunk chunk = SourceChunk(current);
        int end = current.Offset + current.Length;
        int seen = 0;

        for (int i = current.Offset; i < end; ++i)
        {
            if (chunk.Data[i] == (byte)'\n')
            {
                ++seen;
                if (seen == nth)
                {
                    return i - current.Offset;
                }
            }
        }

        return current.Length;
    }

    private static int CountNewlines(byte[] source, int offset, int length)
    {
        int count = 0;
        int end = offset + length;
        for (int i = offset; i < end; ++i)
        {
            if (source[i] == (byte)'\n')
            {
                ++count;
            }
        }
        return count;
    }

    private ulong NextPriority()
    {
        // splitmix64: deterministic and well distributed.
        _randomState += 0x9E3779B97F4A7C15UL;
        ulong z = _randomState;
        z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9UL;
        z = (z ^ (z >> 27)) * 0x94D049BB133111EBUL;
        return z ^ (z >> 31);
    }
}

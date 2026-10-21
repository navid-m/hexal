using System.Runtime.CompilerServices;

namespace Hexal.Util;

/// <summary>
/// A small, reusable growable byte buffer. It is deliberately not thread-safe:
/// the editor is single-threaded and uses these buffers to avoid per-frame and
/// per-keystroke allocations.
/// </summary>
public sealed class ByteBuffer
{
    private byte[] _data;
    private int _length;

    public ByteBuffer(int initialCapacity = 64)
    {
        _data = new byte[Math.Max(1, initialCapacity)];
    }

    public int Length => _length;

    public ReadOnlySpan<byte> Span => _data.AsSpan(0, _length);

    public Span<byte> WritableSpan => _data.AsSpan(0, _length);

    public ref byte this[int index] => ref _data[index];

    public void Clear() => _length = 0;

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void Append(byte value)
    {
        EnsureCapacity(_length + 1);
        _data[_length++] = value;
    }

    public void Append(ReadOnlySpan<byte> value)
    {
        if (value.IsEmpty)
        {
            return;
        }

        EnsureCapacity(_length + value.Length);
        value.CopyTo(_data.AsSpan(_length));
        _length += value.Length;
    }

    /// <summary>
    /// Prepends bytes. Backspace grouping needs this: extending a deletion to
    /// the left is O(n) in the C++ implementation too, and deletion runs are
    /// normally short.
    /// </summary>
    public void Prepend(ReadOnlySpan<byte> value)
    {
        if (value.IsEmpty)
        {
            return;
        }

        EnsureCapacity(_length + value.Length);
        if (_length > 0)
        {
            _data.AsSpan(0, _length).CopyTo(_data.AsSpan(value.Length));
        }
        value.CopyTo(_data);
        _length += value.Length;
    }

    public void EnsureCapacity(int capacity)
    {
        if (capacity <= _data.Length)
        {
            return;
        }

        int newCapacity = _data.Length;
        while (newCapacity < capacity)
        {
            newCapacity = newCapacity > int.MaxValue / 2 ? int.MaxValue : newCapacity * 2;
        }

        Array.Resize(ref _data, newCapacity);
    }

    public byte[] ToArray() => _data.AsSpan(0, _length).ToArray();

    public override string ToString() => System.Text.Encoding.UTF8.GetString(Span);
}

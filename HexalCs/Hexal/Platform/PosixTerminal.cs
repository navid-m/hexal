using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using System.Text;

namespace Hexal.Platform;

[UnsupportedOSPlatform("windows")]
internal sealed unsafe partial class PosixTerminal : ITerminal
{
    private const int StdinFd = 0;
    private const int TcsFlush = 2;
    private const int PollIn = 0x0001;
    private const int PollErr = 0x0008;
    private const int PollHup = 0x0010;
    private const int PollNval = 0x0020;
    private const int Eintr = 4;

    // Linux termios flags.
    private const uint LIgnbrk = 0x0000001;
    private const uint LBrkint = 0x0000002;
    private const uint LParMrk = 0x0000008;
    private const uint LIstrip = 0x0000020;
    private const uint LInlcr = 0x0000040;
    private const uint LIgncr = 0x0000080;
    private const uint LIcrnl = 0x0000100;
    private const uint LIxon = 0x0000400;
    private const uint LOpost = 0x0000001;
    private const uint LEcho = 0x0000008;
    private const uint LEchonl = 0x0000040;
    private const uint LIcanon = 0x0000002;
    private const uint LIsig = 0x0000001;
    private const uint LIexten = 0x0008000;
    private const uint LCsize = 0x0000030;
    private const uint LParenb = 0x0000100;
    private const uint LCs8 = 0x0000030;
    private const int LVmin = 6;
    private const int LVtime = 5;

    // Darwin termios flags. The values differ from Linux and the struct uses
    // unsigned long for the flag fields.
    private const ulong MIgnbrk = 0x0000001;
    private const ulong MBrkint = 0x0000002;
    private const ulong MParMrk = 0x0000008;
    private const ulong MIstrip = 0x0000020;
    private const ulong MInlcr = 0x00000040;
    private const ulong MIgncr = 0x00000080;
    private const ulong MIcrnl = 0x00000100;
    private const ulong MIxon = 0x00000200;
    private const ulong MOpost = 0x00000001;
    private const ulong MEcho = 0x00000008;
    private const ulong MEchonl = 0x00000010;
    private const ulong MIcanon = 0x00000100;
    private const ulong MIsig = 0x00000080;
    private const ulong MIexten = 0x00000400;
    private const ulong MCsize = 0x00000300;
    private const ulong MParenb = 0x00001000;
    private const ulong MCs8 = 0x00000300;
    private const int MVmin = 16;
    private const int MVtime = 17;

    [StructLayout(LayoutKind.Sequential)]
    private unsafe struct TermiosLinux
    {
        public uint c_iflag;
        public uint c_oflag;
        public uint c_cflag;
        public uint c_lflag;
        public byte c_line;
        public fixed byte c_cc[32];
        public uint c_ispeed;
        public uint c_ospeed;
    }

    [StructLayout(LayoutKind.Sequential)]
    private unsafe struct TermiosMac
    {
        public ulong c_iflag;
        public ulong c_oflag;
        public ulong c_cflag;
        public ulong c_lflag;
        public fixed byte c_cc[20];
        public ulong c_ispeed;
        public ulong c_ospeed;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct PollFd
    {
        public int Fd;
        public short Events;
        public short Revents;
    }

    [LibraryImport("libc", EntryPoint = "tcgetattr", SetLastError = true)]
    private static partial int TcGetAttrLinux(int fd, out TermiosLinux termios);

    [LibraryImport("libc", EntryPoint = "tcgetattr", SetLastError = true)]
    private static partial int TcGetAttrMac(int fd, out TermiosMac termios);

    [LibraryImport("libc", EntryPoint = "tcsetattr", SetLastError = true)]
    private static partial int TcSetAttrLinux(int fd, int optionalActions, ref TermiosLinux termios);

    [LibraryImport("libc", EntryPoint = "tcsetattr", SetLastError = true)]
    private static partial int TcSetAttrMac(int fd, int optionalActions, ref TermiosMac termios);

    [LibraryImport("libc", EntryPoint = "poll", SetLastError = true)]
    private static partial int Poll(ref PollFd fds, nuint nfds, int timeout);

    [LibraryImport("libc", EntryPoint = "read", SetLastError = true)]
    private static unsafe partial nint Read(int fd, byte* buffer, nuint count);

    private readonly bool _isMac = OperatingSystem.IsMacOS();

    private TermiosLinux _originalLinux;
    private TermiosMac _originalMac;
    private Stream? _output;
    private PosixSignalRegistration? _resizeRegistration;
    private int _resizeRequested;
    private int _pendingByte = -1;
    private bool _rawMode;
    private bool _initialized;

    public bool Init(out string error)
    {
        error = string.Empty;

        if (Console.IsInputRedirected || Console.IsOutputRedirected)
        {
            error = "standard input/output is not an interactive terminal";
            return false;
        }

        if (_isMac)
        {
            if (TcGetAttrMac(StdinFd, out _originalMac) != 0)
            {
                error = $"cannot read terminal attributes: {ErrnoMessage()}";
                return false;
            }

            TermiosMac raw = _originalMac;
            raw.c_iflag &= ~(MIgnbrk | MBrkint | MParMrk | MIstrip | MInlcr | MIgncr |
                             MIcrnl | MIxon);
            raw.c_oflag &= ~MOpost;
            raw.c_lflag &= ~(MEcho | MEchonl | MIcanon | MIsig | MIexten);
            raw.c_cflag &= ~(MCsize | MParenb);
            raw.c_cflag |= MCs8;
            raw.c_cc[MVmin] = 1;
            raw.c_cc[MVtime] = 0;

            if (TcSetAttrMac(StdinFd, TcsFlush, ref raw) != 0)
            {
                error = $"cannot configure terminal: {ErrnoMessage()}";
                return false;
            }
        }
        else
        {
            if (TcGetAttrLinux(StdinFd, out _originalLinux) != 0)
            {
                error = $"cannot read terminal attributes: {ErrnoMessage()}";
                return false;
            }

            TermiosLinux raw = _originalLinux;
            raw.c_iflag &= ~(LIgnbrk | LBrkint | LParMrk | LIstrip | LInlcr | LIgncr |
                             LIcrnl | LIxon);
            raw.c_oflag &= ~LOpost;
            raw.c_lflag &= ~(LEcho | LEchonl | LIcanon | LIsig | LIexten);
            raw.c_cflag &= ~(LCsize | LParenb);
            raw.c_cflag |= LCs8;
            raw.c_cc[LVmin] = 1;
            raw.c_cc[LVtime] = 0;

            if (TcSetAttrLinux(StdinFd, TcsFlush, ref raw) != 0)
            {
                error = $"cannot configure terminal: {ErrnoMessage()}";
                return false;
            }
        }

        _rawMode = true;

        try
        {
            _resizeRegistration = PosixSignalRegistration.Create(PosixSignal.SIGWINCH,
                context =>
                {
                    context.Cancel = true;
                    Volatile.Write(ref _resizeRequested, 1);
                });
        }
        catch (Exception ex) when (ex is PlatformNotSupportedException or IOException)
        {
            error = $"cannot install resize handler: {ex.Message}";
            Shutdown();
            return false;
        }

        _output = Console.OpenStandardOutput();
        // 1004 enables terminal focus reporting so the editor can check for
        // external file changes when the window regains focus. 1000/1006
        // enable button and wheel reporting using the extended SGR encoding.
        Write("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H\x1b[?1004h\x1b[?1000h\x1b[?1002h\x1b[?1006h"u8);
        _initialized = true;
        return true;
    }

    public void Shutdown()
    {
        if (_initialized)
        {
            Write("\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?1004l\x1b[0m\x1b[?25h\x1b[?1049l"u8);
        }

        if (_output is not null)
        {
            try
            {
                _output.Flush();
            }
            catch (IOException)
            {
                // Ignore shutdown I/O failures.
            }
            _output = null;
        }

        if (_rawMode)
        {
            if (_isMac)
            {
                TermiosMac original = _originalMac;
                TcSetAttrMac(StdinFd, TcsFlush, ref original);
            }
            else
            {
                TermiosLinux original = _originalLinux;
                TcSetAttrLinux(StdinFd, TcsFlush, ref original);
            }
            _rawMode = false;
        }

        _resizeRegistration?.Dispose();
        _resizeRegistration = null;
        _initialized = false;
    }

    public bool ReadEvent(out InputEvent inputEvent, int timeoutMilliseconds)
    {
        inputEvent = default;

        while (true)
        {
            if (Volatile.Read(ref _resizeRequested) != 0)
            {
                Volatile.Write(ref _resizeRequested, 0);
                inputEvent.Key = Key.Resize;
                return true;
            }

            int value;
            if (_pendingByte >= 0)
            {
                value = _pendingByte;
                _pendingByte = -1;
            }
            else
            {
                value = ReadByte(timeoutMilliseconds);
                if (value == -2)
                {
                    continue;
                }
                if (value == -3)
                {
                    inputEvent.Key = Key.None;
                    return true;
                }
                if (value < 0)
                {
                    return false;
                }
            }

            if (value == 0x1B)
            {
                return ReadEscape(out inputEvent);
            }

            return DecodeInputByte(value, out inputEvent);
        }
    }

    public void Write(ReadOnlySpan<byte> text)
    {
        if (_output is null || text.IsEmpty)
        {
            return;
        }

        try
        {
            _output.Write(text);
            _output.Flush();
        }
        catch (IOException)
        {
            // The editor loop will end when input is no longer available.
        }
    }

    public void GetSize(out int columns, out int rows)
    {
        try
        {
            columns = Console.WindowWidth;
            rows = Console.WindowHeight;
            if (columns > 0 && rows > 0)
            {
                return;
            }
        }
        catch (IOException)
        {
            // Fall through to the conventional default.
        }

        columns = 80;
        rows = 24;
    }

    public void Dispose() => Shutdown();

    private int ReadByte(int timeoutMilliseconds)
    {
        byte value;

        while (true)
        {
            if (Volatile.Read(ref _resizeRequested) != 0)
            {
                return -2;
            }

            var descriptor = new PollFd { Fd = StdinFd, Events = PollIn };
            int ready = Poll(ref descriptor, 1, timeoutMilliseconds < 0 ? 100 : timeoutMilliseconds);

            if (ready < 0)
            {
                int errno = Marshal.GetLastPInvokeError();
                return errno == Eintr ? -2 : -1;
            }

            if (ready == 0)
            {
                if (timeoutMilliseconds < 0)
                {
                    continue;
                }
                return -3;
            }

            if ((descriptor.Revents & PollIn) != 0)
            {
                nint count;
                unsafe
                {
                    do
                    {
                        count = Read(StdinFd, &value, 1);
                    }
                    while (count < 0 && Marshal.GetLastPInvokeError() == Eintr);
                }

                return count == 1 ? value : -1;
            }

            if ((descriptor.Revents & (PollErr | PollHup | PollNval)) != 0)
            {
                return -1;
            }

            if (timeoutMilliseconds >= 0)
            {
                return -1;
            }
        }
    }

    private bool DecodeInputByte(int value, out InputEvent inputEvent)
    {
        inputEvent = default;

        if (value == '\r' || value == '\n')
        {
            inputEvent.Key = Key.Enter;
            return true;
        }
        if (value == 0x7F || value == 0x08)
        {
            inputEvent.Key = Key.Backspace;
            return true;
        }
        if (value == 0x09)
        {
            inputEvent.Key = Key.Tab;
            return true;
        }
        if (value >= 0x01 && value <= 0x1A)
        {
            inputEvent.Key = Key.Character;
            inputEvent.Character = new Rune('a' + (value - 1));
            inputEvent.Ctrl = true;
            return true;
        }
        if (value < 0x20)
        {
            inputEvent.Key = Key.Unknown;
            return true;
        }
        if (value < 0x80)
        {
            inputEvent.Key = Key.Character;
            inputEvent.Character = new Rune(value);
            return true;
        }

        // Multi-byte UTF-8 input.
        Span<byte> bytes = stackalloc byte[4];
        bytes[0] = (byte)value;

        int needed = 1;
        if ((value & 0xE0) == 0xC0)
        {
            needed = 2;
        }
        else if ((value & 0xF0) == 0xE0)
        {
            needed = 3;
        }
        else if ((value & 0xF8) == 0xF0)
        {
            needed = 4;
        }

        int count = 1;
        while (count < needed)
        {
            int next = ReadByte(50);
            if (next < 0)
            {
                break;
            }
            bytes[count++] = (byte)next;
        }

        Rune rune;
        if (Rune.DecodeFromUtf8(bytes[..count], out rune, out int consumed) !=
                System.Buffers.OperationStatus.Done ||
            consumed != count)
        {
            rune = Rune.ReplacementChar;
        }

        inputEvent.Key = Key.Character;
        inputEvent.Character = rune;
        return true;
    }

    private bool ReadEscape(out InputEvent inputEvent)
    {
        inputEvent = default;

        int next = ReadByte(25);
        if (next == -2)
        {
            inputEvent.Key = Key.Resize;
            return true;
        }
        if (next < 0)
        {
            inputEvent.Key = Key.Escape;
            return true;
        }

        if (next != '[' && next != 'O')
        {
            // A bare ESC followed quickly by another key. Process the following
            // byte as a separate key on the next call.
            _pendingByte = next;
            inputEvent.Key = Key.Escape;
            return true;
        }

        bool csi = next == '[';
        Span<char> parameters = stackalloc char[32];
        int parameterLength = 0;
        int final = 0;

        for (int i = 0; i < 32; ++i)
        {
            int value = ReadByte(25);
            if (value < 0)
            {
                inputEvent.Key = value == -2 ? Key.Resize : Key.Escape;
                return true;
            }
            if (value >= 0x40 && value <= 0x7E)
            {
                final = value;
                break;
            }
            if (parameterLength < 30)
            {
                parameters[parameterLength++] = (char)value;
            }
        }

        if (final == 0)
        {
            inputEvent.Key = Key.Unknown;
            return true;
        }

        if (csi && parameterLength > 0 && parameters[0] == '<')
        {
            inputEvent = DecodeMouse(parameters[1..parameterLength], final);
            return true;
        }

        if (!csi)
        {
            inputEvent.Key = final switch
            {
                'A' => Key.Up,
                'B' => Key.Down,
                'C' => Key.Right,
                'D' => Key.Left,
                'H' => Key.Home,
                'F' => Key.End,
                'P' => Key.F1,
                'Q' => Key.F2,
                'R' => Key.F3,
                'S' => Key.F4,
                _ => Key.Unknown
            };
            return true;
        }

        ApplyModifiers(parameters[..parameterLength], ref inputEvent);

        inputEvent.Key = final switch
        {
            'A' => Key.Up,
            'B' => Key.Down,
            'C' => Key.Right,
            'D' => Key.Left,
            'H' => Key.Home,
            'F' => Key.End,
            'I' => Key.FocusIn,
            'O' => Key.FocusOut,
            'Z' => Key.BackTab,
            '~' => DecodeTilde(parameters[..parameterLength]),
            _ => Key.Unknown
        };
        return true;
    }

    private static InputEvent DecodeMouse(ReadOnlySpan<char> parameters, int final)
    {
        var inputEvent = new InputEvent();
        int index = 0;
        int button = ReadMouseNumber(parameters, ref index);
        int x = ReadMouseNumber(parameters, ref index);
        int y = ReadMouseNumber(parameters, ref index);

        inputEvent.MouseX = Math.Max(0, x - 1);
        inputEvent.MouseY = Math.Max(0, y - 1);
        inputEvent.Shift = (button & 4) != 0;
        inputEvent.Alt = (button & 8) != 0;
        inputEvent.Ctrl = (button & 16) != 0;

        int buttonNumber = button & 3;
        if ((button & 64) != 0)
        {
            inputEvent.Key = buttonNumber switch
            {
                0 => Key.ScrollUp,
                1 => Key.ScrollDown,
                _ => Key.Unknown
            };
            return inputEvent;
        }

        inputEvent.MouseButton = buttonNumber switch
        {
            0 => MouseButton.Left,
            1 => MouseButton.Middle,
            2 => MouseButton.Right,
            _ => MouseButton.None
        };

        if (final != 'm' && (button & 32) != 0)
        {
            inputEvent.Key = Key.MouseMove;
            return inputEvent;
        }

        inputEvent.Key = final == 'm' ? Key.MouseUp : Key.MouseDown;
        return inputEvent;
    }

    private static int ReadMouseNumber(ReadOnlySpan<char> parameters, ref int index)
    {
        int value = 0;
        bool found = false;

        while (index < parameters.Length && parameters[index] >= '0' &&
               parameters[index] <= '9')
        {
            value = (value * 10) + (parameters[index] - '0');
            found = true;
            ++index;
        }

        if (index < parameters.Length && parameters[index] == ';')
        {
            ++index;
        }

        return found ? value : 0;
    }

    private static void ApplyModifiers(ReadOnlySpan<char> parameters, ref InputEvent inputEvent)
    {
        int semicolon = parameters.LastIndexOf(';');
        if (semicolon < 0)
        {
            return;
        }

        int modifier = 0;
        for (int i = semicolon + 1; i < parameters.Length; ++i)
        {
            char c = parameters[i];
            if (c < '0' || c > '9')
            {
                break;
            }
            modifier = (modifier * 10) + (c - '0');
        }

        // xterm modifier encoding: 1 + shift(1) + alt(2) + ctrl(4).
        int mask = modifier - 1;
        if (mask > 0)
        {
            inputEvent.Shift = (mask & 1) != 0;
            inputEvent.Alt = (mask & 2) != 0;
            inputEvent.Ctrl = (mask & 4) != 0;
        }
    }

    private static Key DecodeTilde(ReadOnlySpan<char> parameters)
    {
        int code = 0;
        for (int i = 0; i < parameters.Length; ++i)
        {
            char c = parameters[i];
            if (c < '0' || c > '9')
            {
                break;
            }
            code = (code * 10) + (c - '0');
        }

        return code switch
        {
            1 or 7 => Key.Home,
            4 or 8 => Key.End,
            2 => Key.Insert,
            3 => Key.Delete,
            5 => Key.PageUp,
            6 => Key.PageDown,
            11 => Key.F1,
            12 => Key.F2,
            13 => Key.F3,
            14 => Key.F4,
            15 => Key.F5,
            17 => Key.F6,
            18 => Key.F7,
            19 => Key.F8,
            20 => Key.F9,
            21 => Key.F10,
            23 => Key.F11,
            24 => Key.F12,
            _ => Key.Unknown
        };
    }

    private static string ErrnoMessage()
        => new Win32Exception(Marshal.GetLastPInvokeError()).Message;
}

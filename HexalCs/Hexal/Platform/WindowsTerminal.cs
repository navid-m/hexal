using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Hexal.Platform;

internal sealed partial class WindowsTerminal : ITerminal
{
    private const int StdInputHandle = -10;
    private const int StdOutputHandle = -11;

    private const uint GenericRead = 0x80000000;
    private const uint GenericWrite = 0x40000000;
    private const uint FileShareRead = 0x00000001;
    private const uint FileShareWrite = 0x00000002;
    private const uint OpenExisting = 3;

    private const uint EnableProcessedInput = 0x0001;
    private const uint EnableLineInput = 0x0002;
    private const uint EnableEchoInput = 0x0004;
    private const uint EnableWindowInput = 0x0008;
    private const uint EnableProcessedOutput = 0x0001;
    private const uint EnableVirtualTerminalProcessing = 0x0004;
    private const uint DisableNewlineAutoReturn = 0x0008;
    private const uint EnableQuickEditMode = 0x0040;
    private const uint EnableExtendedFlags = 0x0080;
    private const uint EnableMouseInput = 0x0010;

    private const uint LeftAltPressed = 0x0001;
    private const uint LeftCtrlPressed = 0x0002;
    private const uint RightAltPressed = 0x0004;
    private const uint RightCtrlPressed = 0x0008;
    private const uint ShiftPressed = 0x0010;

    private const ushort KeyEventType = 0x0001;
    private const ushort MouseEventType = 0x0002;
    private const ushort WindowBufferSizeEventType = 0x0004;
    private const ushort FocusEventType = 0x0010;

    private const uint MouseMoved = 0x0001;
    private const uint MouseWheeled = 0x0004;
    private const uint MouseHorizontalWheeled = 0x0008;
    private const uint LeftButtonPressed = 0x0001;

    private const uint WaitTimeout = 0x00000102;
    private const uint WaitFailed = 0xFFFFFFFF;

    private const ushort VkBack = 0x08;
    private const ushort VkTab = 0x09;
    private const ushort VkReturn = 0x0D;
    private const ushort VkEscape = 0x1B;
    private const ushort VkPrior = 0x21;
    private const ushort VkNext = 0x22;
    private const ushort VkEnd = 0x23;
    private const ushort VkHome = 0x24;
    private const ushort VkLeft = 0x25;
    private const ushort VkUp = 0x26;
    private const ushort VkRight = 0x27;
    private const ushort VkDown = 0x28;
    private const ushort VkInsert = 0x2D;
    private const ushort VkDelete = 0x2E;
    private const ushort VkF1 = 0x70;

    [StructLayout(LayoutKind.Sequential)]
    private struct KeyEventRecord
    {
        public int KeyDown;
        public ushort RepeatCount;
        public ushort VirtualKeyCode;
        public ushort VirtualScanCode;
        public char UnicodeChar;
        public uint ControlKeyState;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct WindowBufferSizeRecord
    {
        public short X;
        public short Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Coord
    {
        public short X;
        public short Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct SmallRect
    {
        public short Left;
        public short Top;
        public short Right;
        public short Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ConsoleScreenBufferInfo
    {
        public Coord Size;
        public Coord CursorPosition;
        public ushort Attributes;
        public SmallRect Window;
        public Coord MaximumWindowSize;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MousePositionRecord
    {
        public short X;
        public short Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MouseEventRecord
    {
        public MousePositionRecord MousePosition;
        public uint ButtonState;
        public uint ControlKeyState;
        public uint EventFlags;
    }

    [StructLayout(LayoutKind.Explicit, Size = 20)]
    private struct InputRecord
    {
        [FieldOffset(0)]
        public ushort EventType;

        [FieldOffset(4)]
        public KeyEventRecord KeyEvent;

        [FieldOffset(4)]
        public MouseEventRecord MouseEvent;

        [FieldOffset(4)]
        public WindowBufferSizeRecord WindowBufferSizeEvent;

        [FieldOffset(4)]
        public int FocusSet;
    }

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial IntPtr GetStdHandle(int stdHandle);

    [LibraryImport(
        "kernel32.dll",
        EntryPoint = "CreateFileW",
        SetLastError = true,
        StringMarshalling = StringMarshalling.Utf16
    )]
    private static partial IntPtr CreateFile(
        string fileName,
        uint desiredAccess,
        uint shareMode,
        IntPtr securityAttributes,
        uint creationDisposition,
        uint flagsAndAttributes,
        IntPtr templateFile
    );

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial int CloseHandle(IntPtr handle);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial int GetConsoleScreenBufferInfo(
        IntPtr consoleHandle,
        out ConsoleScreenBufferInfo info
    );

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial int GetConsoleMode(IntPtr consoleHandle, out uint mode);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial int SetConsoleMode(IntPtr consoleHandle, uint mode);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial int ReadConsoleInputW(
        IntPtr consoleHandle,
        ref InputRecord inputRecord,
        uint length,
        out uint numberOfEventsRead
    );

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial uint WaitForSingleObject(IntPtr handle, uint milliseconds);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial uint GetConsoleCP();

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial uint GetConsoleOutputCP();

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial int SetConsoleCP(uint codePage);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial int SetConsoleOutputCP(uint codePage);

    private IntPtr _inputHandle;
    private IntPtr _outputHandle;
    private bool _ownsInputHandle;
    private bool _ownsOutputHandle;
    private uint _originalInputMode;
    private uint _originalOutputMode;
    private uint _originalInputCodePage;
    private uint _originalOutputCodePage;
    private Stream? _output;
    private bool _initialized;

    private char _pendingHighSurrogate;
    private KeyEventRecord _stashedKey;
    private bool _hasStashedKey;
    private InputEvent _repeatEvent;
    private ushort _repeatRemaining;
    private bool _mouseLeftDown;

    private static IntPtr CreateConsoleFile(string name)
    {
        return CreateFile(
            name,
            GenericRead | GenericWrite,
            FileShareRead | FileShareWrite,
            IntPtr.Zero,
            OpenExisting,
            0,
            IntPtr.Zero
        );
    }

    private static bool IsInvalidHandle(IntPtr handle) =>
        handle == IntPtr.Zero || handle == new IntPtr(-1);

    public bool Init(out string error)
    {
        error = string.Empty;

        if (Console.IsInputRedirected || Console.IsOutputRedirected)
        {
            // Git and other tools can spawn an editor with redirected standard
            // handles while a console is still attached. CONIN$/CONOUT$ give
            // direct access to that console.
            _inputHandle = CreateConsoleFile("CONIN$");
            if (IsInvalidHandle(_inputHandle))
            {
                error =
                    "standard input/output is not an interactive terminal: "
                    + $"cannot open CONIN$ ({Win32Message()})";
                return false;
            }

            _outputHandle = CreateConsoleFile("CONOUT$");
            if (IsInvalidHandle(_outputHandle))
            {
                CloseHandle(_inputHandle);
                _inputHandle = IntPtr.Zero;
                error =
                    "standard input/output is not an interactive terminal: "
                    + $"cannot open CONOUT$ ({Win32Message()})";
                return false;
            }

            _ownsInputHandle = true;
            _ownsOutputHandle = true;
        }
        else
        {
            _inputHandle = GetStdHandle(StdInputHandle);
            _outputHandle = GetStdHandle(StdOutputHandle);
        }

        if (IsInvalidHandle(_inputHandle) || IsInvalidHandle(_outputHandle))
        {
            error = $"cannot access the console handles: {Win32Message()}";
            return false;
        }

        if (GetConsoleMode(_inputHandle, out _originalInputMode) == 0)
        {
            error = $"cannot read the console input mode: {Win32Message()}";
            return false;
        }

        if (GetConsoleMode(_outputHandle, out _originalOutputMode) == 0)
        {
            error = $"cannot read the console output mode: {Win32Message()}";
            return false;
        }

        uint inputMode =
            (
                _originalInputMode
                & ~(EnableEchoInput | EnableLineInput | EnableProcessedInput | EnableQuickEditMode)
            )
            | EnableWindowInput
            | EnableExtendedFlags
            | EnableMouseInput;

        if (SetConsoleMode(_inputHandle, inputMode) == 0)
        {
            error = $"cannot configure the console input mode: {Win32Message()}";
            return false;
        }

        uint outputMode =
            _originalOutputMode
            | EnableProcessedOutput
            | EnableVirtualTerminalProcessing
            | DisableNewlineAutoReturn;

        if (SetConsoleMode(_outputHandle, outputMode) == 0)
        {
            SetConsoleMode(_inputHandle, _originalInputMode);
            error = "Windows 10 or newer with virtual terminal support is required";
            return false;
        }

        _originalInputCodePage = GetConsoleCP();
        _originalOutputCodePage = GetConsoleOutputCP();
        SetConsoleCP(65001);
        SetConsoleOutputCP(65001);

        _output = _ownsOutputHandle
            ? new FileStream(
                new SafeFileHandle(_outputHandle, ownsHandle: false),
                FileAccess.Write,
                bufferSize: 1
            )
            : Console.OpenStandardOutput();
        Write("\x1b[?1049h\x1b[?25l\x1b[2J\x1b[H"u8);
        _initialized = true;
        return true;
    }

    public void Shutdown()
    {
        if (_initialized)
        {
            Write("\x1b[0m\x1b[?25h\x1b[?1049l"u8);
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

        if (_initialized)
        {
            SetConsoleMode(_inputHandle, _originalInputMode);
            SetConsoleMode(_outputHandle, _originalOutputMode);
            SetConsoleCP(_originalInputCodePage);
            SetConsoleOutputCP(_originalOutputCodePage);
        }

        if (_ownsInputHandle && !IsInvalidHandle(_inputHandle))
        {
            CloseHandle(_inputHandle);
        }
        if (_ownsOutputHandle && !IsInvalidHandle(_outputHandle))
        {
            CloseHandle(_outputHandle);
        }

        _inputHandle = IntPtr.Zero;
        _outputHandle = IntPtr.Zero;
        _ownsInputHandle = false;
        _ownsOutputHandle = false;
        _initialized = false;
    }

    public bool ReadEvent(out InputEvent inputEvent, int timeoutMilliseconds)
    {
        inputEvent = default;

        while (true)
        {
            if (_repeatRemaining > 0)
            {
                --_repeatRemaining;
                inputEvent = _repeatEvent;
                return true;
            }

            KeyEventRecord key;
            if (_hasStashedKey)
            {
                key = _stashedKey;
                _hasStashedKey = false;
            }
            else
            {
                if (
                    !ReadKeyEvent(
                        out InputEvent special,
                        out bool hasSpecial,
                        out key,
                        timeoutMilliseconds
                    )
                )
                {
                    return false;
                }

                if (hasSpecial)
                {
                    inputEvent = special;
                    return true;
                }
            }

            char character = key.UnicodeChar;

            if (_pendingHighSurrogate != 0)
            {
                char high = _pendingHighSurrogate;
                _pendingHighSurrogate = '\0';

                if (char.IsLowSurrogate(character))
                {
                    int codePoint = char.ConvertToUtf32(high, character);
                    inputEvent.Key = Key.Character;
                    inputEvent.Character = new Rune(codePoint);
                    ApplyModifiers(key.ControlKeyState, ref inputEvent);
                    StoreRepeat(key.RepeatCount, inputEvent);
                    return true;
                }

                // The high surrogate was not followed by a low surrogate. Emit
                // U+FFFD and process the current key on the next call.
                _stashedKey = key;
                _hasStashedKey = true;
                inputEvent.Key = Key.Character;
                inputEvent.Character = Rune.ReplacementChar;
                return true;
            }

            if (char.IsHighSurrogate(character))
            {
                _pendingHighSurrogate = character;
                continue;
            }

            inputEvent = DecodeKey(key);

            if (char.IsLowSurrogate(character))
            {
                inputEvent.Key = Key.Character;
                inputEvent.Character = Rune.ReplacementChar;
            }

            StoreRepeat(key.RepeatCount, inputEvent);
            return true;
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
        if (!_ownsOutputHandle)
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
        }
        else if (
            !IsInvalidHandle(_outputHandle)
            && GetConsoleScreenBufferInfo(_outputHandle, out ConsoleScreenBufferInfo info) != 0
        )
        {
            columns = info.Window.Right - info.Window.Left + 1;
            rows = info.Window.Bottom - info.Window.Top + 1;
            if (columns > 0 && rows > 0)
            {
                return;
            }
        }

        columns = 80;
        rows = 24;
    }

    public void Dispose() => Shutdown();

    private bool ReadKeyEvent(
        out InputEvent special,
        out bool hasSpecial,
        out KeyEventRecord key,
        int timeoutMilliseconds
    )
    {
        special = default;
        hasSpecial = false;
        key = default;

        while (true)
        {
            if (timeoutMilliseconds >= 0)
            {
                uint wait = WaitForSingleObject(_inputHandle, (uint)timeoutMilliseconds);
                if (wait == WaitTimeout)
                {
                    special.Key = Key.None;
                    hasSpecial = true;
                    return true;
                }
                if (wait == WaitFailed)
                {
                    return false;
                }
            }

            InputRecord record = default;
            if (ReadConsoleInputW(_inputHandle, ref record, 1, out uint read) == 0 || read == 0)
            {
                return false;
            }

            if (record.EventType == WindowBufferSizeEventType)
            {
                special.Key = Key.Resize;
                hasSpecial = true;
                return true;
            }

            if (record.EventType == FocusEventType)
            {
                special.Key = record.FocusSet != 0 ? Key.FocusIn : Key.FocusOut;
                hasSpecial = true;
                return true;
            }

            if (record.EventType == MouseEventType)
            {
                if (TryDecodeMouse(record.MouseEvent, out special))
                {
                    hasSpecial = true;
                    return true;
                }
                continue;
            }

            if (record.EventType != KeyEventType)
            {
                continue;
            }

            if (record.KeyEvent.KeyDown == 0)
            {
                continue;
            }

            key = record.KeyEvent;
            return true;
        }
    }

    private bool TryDecodeMouse(MouseEventRecord mouse, out InputEvent inputEvent)
    {
        inputEvent = default;
        if ((mouse.EventFlags & MouseHorizontalWheeled) != 0)
        {
            return false;
        }

        inputEvent.MouseX = Math.Max(0, (int)mouse.MousePosition.X);
        inputEvent.MouseY = Math.Max(0, (int)mouse.MousePosition.Y);
        ApplyModifiers(mouse.ControlKeyState, ref inputEvent);

        if ((mouse.EventFlags & MouseWheeled) != 0)
        {
            short delta = (short)((mouse.ButtonState >> 16) & 0xFFFF);
            if (delta == 0)
            {
                return false;
            }

            inputEvent.Key = delta > 0 ? Key.ScrollUp : Key.ScrollDown;
            return true;
        }

        uint buttons = mouse.ButtonState;
        bool leftDown = (buttons & LeftButtonPressed) != 0;
        if ((mouse.EventFlags & MouseMoved) != 0)
        {
            if (_mouseLeftDown && leftDown)
            {
                inputEvent.Key = Key.MouseMove;
                inputEvent.MouseButton = MouseButton.Left;
                return true;
            }
            return false;
        }

        if (leftDown && !_mouseLeftDown)
        {
            _mouseLeftDown = true;
            inputEvent.Key = Key.MouseDown;
            inputEvent.MouseButton = MouseButton.Left;
            return true;
        }

        if (!leftDown && _mouseLeftDown)
        {
            _mouseLeftDown = false;
            inputEvent.Key = Key.MouseUp;
            inputEvent.MouseButton = MouseButton.Left;
            return true;
        }

        return false;
    }

    private static InputEvent DecodeKey(KeyEventRecord key)
    {
        var result = new InputEvent();
        ApplyModifiers(key.ControlKeyState, ref result);

        bool ctrl = result.Ctrl;
        bool alt = result.Alt;
        char character = key.UnicodeChar;

        switch (key.VirtualKeyCode)
        {
            case VkReturn:
                result.Key = Key.Enter;
                return result;
            case VkEscape:
                result.Key = Key.Escape;
                return result;
            case VkBack:
                result.Key = Key.Backspace;
                return result;
            case VkTab:
                result.Key = result.Shift ? Key.BackTab : Key.Tab;
                return result;
            case VkDelete:
                result.Key = Key.Delete;
                return result;
            case VkLeft:
                result.Key = Key.Left;
                return result;
            case VkRight:
                result.Key = Key.Right;
                return result;
            case VkUp:
                result.Key = Key.Up;
                return result;
            case VkDown:
                result.Key = Key.Down;
                return result;
            case VkHome:
                result.Key = Key.Home;
                return result;
            case VkEnd:
                result.Key = Key.End;
                return result;
            case VkPrior:
                result.Key = Key.PageUp;
                return result;
            case VkNext:
                result.Key = Key.PageDown;
                return result;
            case VkInsert:
                result.Key = Key.Insert;
                return result;
        }

        if (key.VirtualKeyCode >= VkF1 && key.VirtualKeyCode <= VkF1 + 11)
        {
            result.Key = (Key)((int)Key.F1 + (key.VirtualKeyCode - VkF1));
            return result;
        }

        // Ctrl+[ is the terminal Escape key on POSIX and should behave the same
        // way here.
        if (ctrl && character == 0x1B)
        {
            result.Key = Key.Escape;
            return result;
        }

        if (character == '\r' || character == '\n')
        {
            result.Key = Key.Enter;
            return result;
        }

        if (character == '\t')
        {
            result.Key = Key.Tab;
            return result;
        }

        if (character == '\b' || character == 0x7F)
        {
            result.Key = Key.Backspace;
            return result;
        }

        if (ctrl && alt && character >= 0x20)
        {
            // AltGr: a printable character should not be treated as a chord.
            ctrl = false;
            alt = false;
        }

        if (ctrl && !alt && character >= 0x01 && character <= 0x1A)
        {
            result.Key = Key.Character;
            result.Character = new Rune('a' + (character - 1));
            result.Ctrl = true;
            result.Alt = false;
            return result;
        }

        if (character == 0)
        {
            if (ctrl && !alt && key.VirtualKeyCode >= 'A' && key.VirtualKeyCode <= 'Z')
            {
                result.Key = Key.Character;
                result.Character = new Rune('a' + (key.VirtualKeyCode - 'A'));
                result.Ctrl = true;
                result.Alt = false;
                return result;
            }

            result.Key = Key.Unknown;
            return result;
        }

        if (character < 0x20)
        {
            result.Key = Key.Unknown;
            return result;
        }

        result.Key = Key.Character;
        result.Character = new Rune(character);
        result.Ctrl = ctrl;
        result.Alt = alt;
        return result;
    }

    private static void ApplyModifiers(uint state, ref InputEvent inputEvent)
    {
        inputEvent.Ctrl = (state & (LeftCtrlPressed | RightCtrlPressed)) != 0;
        inputEvent.Alt = (state & (LeftAltPressed | RightAltPressed)) != 0;
        inputEvent.Shift = (state & ShiftPressed) != 0;
    }

    private void StoreRepeat(ushort repeatCount, InputEvent inputEvent)
    {
        if (repeatCount <= 1)
        {
            return;
        }

        _repeatEvent = inputEvent;
        _repeatRemaining = (ushort)(repeatCount - 1);
    }

    private static string Win32Message() =>
        new Win32Exception(Marshal.GetLastPInvokeError()).Message;
}

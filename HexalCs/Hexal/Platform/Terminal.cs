using System.Text;

namespace Hexal.Platform;

public enum Key
{
    None,
    Character,
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    MouseDown,
    MouseUp,
    MouseMove,
    ScrollUp,
    ScrollDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    FocusIn,
    FocusOut,
    Resize,
    Unknown
}

public enum MouseButton
{
    None,
    Left,
    Middle,
    Right
}

public struct InputEvent
{
    public Key Key;
    public Rune Character;
    public bool Ctrl;
    public bool Alt;
    public bool Shift;

    /// <summary>Zero-based terminal column for mouse events.</summary>
    public int MouseX;

    /// <summary>Zero-based terminal row for mouse events.</summary>
    public int MouseY;

    public MouseButton MouseButton;
}

/// <summary>
/// Platform terminal abstraction. The POSIX backend uses termios, ANSI
/// sequences and SIGWINCH; the Windows backend uses the console API with
/// ENABLE_VIRTUAL_TERMINAL_PROCESSING for output and ReadConsoleInputW for
/// Unicode keyboard events.
/// </summary>
public interface ITerminal : IDisposable
{
    bool Init(out string error);

    void Shutdown();

    /// <summary>
    /// Reads one event. A negative timeout blocks; a non-negative timeout
    /// returns a Key.None event when no input arrives in time. The editor uses
    /// this to poll background saves without a separate wake-up thread.
    /// </summary>
    bool ReadEvent(out InputEvent inputEvent, int timeoutMilliseconds);

    void Write(ReadOnlySpan<byte> text);

    void GetSize(out int columns, out int rows);
}

public static class TerminalFactory
{
    public static ITerminal Create()
    {
        if (OperatingSystem.IsWindows())
        {
            return new WindowsTerminal();
        }

        return new PosixTerminal();
    }
}

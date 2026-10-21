using System.Text;

namespace Hexal.Platform;

public enum Key
{
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
    Resize,
    Unknown
}

public struct InputEvent
{
    public Key Key;
    public Rune Character;
    public bool Ctrl;
    public bool Alt;
    public bool Shift;
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

    bool ReadEvent(out InputEvent inputEvent);

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

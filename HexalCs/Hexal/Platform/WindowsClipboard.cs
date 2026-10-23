using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace Hexal.Platform;

/// <summary>
/// Minimal Win32 clipboard access. Talking to user32 directly avoids spawning
/// a PowerShell helper, which can fail with "Access is denied" when the editor
/// runs inside Windows Terminal's pseudoconsole.
/// </summary>
internal static partial class WindowsClipboard
{
    private const uint CfUnicodeText = 13;
    private const uint GmemMoveable = 0x0002;
    private const uint GmemZeroInit = 0x0040;
    private const int OpenAttempts = 10;
    private const int OpenRetryDelayMilliseconds = 20;

    [LibraryImport("user32.dll", SetLastError = true)]
    private static partial int OpenClipboard(IntPtr owner);

    [LibraryImport("user32.dll", SetLastError = true)]
    private static partial int CloseClipboard();

    [LibraryImport("user32.dll", SetLastError = true)]
    private static partial int EmptyClipboard();

    [LibraryImport("user32.dll", SetLastError = true)]
    private static partial int IsClipboardFormatAvailable(uint format);

    [LibraryImport("user32.dll", SetLastError = true)]
    private static partial IntPtr GetClipboardData(uint format);

    [LibraryImport("user32.dll", SetLastError = true)]
    private static partial IntPtr SetClipboardData(uint format, IntPtr memory);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial IntPtr GlobalAlloc(uint flags, UIntPtr bytes);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial IntPtr GlobalLock(IntPtr memory);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial int GlobalUnlock(IntPtr memory);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial IntPtr GlobalFree(IntPtr memory);

    public static bool TrySetText(ReadOnlySpan<byte> utf8, out string error)
    {
        error = string.Empty;
        string text = Encoding.UTF8.GetString(utf8);
        byte[] encoded = Encoding.Unicode.GetBytes(text + "\0");

        IntPtr memory = GlobalAlloc(GmemMoveable | GmemZeroInit,
            (UIntPtr)encoded.Length);
        if (memory == IntPtr.Zero)
        {
            error = Win32Message();
            return false;
        }

        IntPtr pointer = GlobalLock(memory);
        if (pointer == IntPtr.Zero)
        {
            error = Win32Message();
            GlobalFree(memory);
            return false;
        }

        try
        {
            Marshal.Copy(encoded, 0, pointer, encoded.Length);
        }
        finally
        {
            GlobalUnlock(memory);
        }

        if (!TryOpen(out error))
        {
            GlobalFree(memory);
            return false;
        }

        try
        {
            if (EmptyClipboard() == 0)
            {
                error = Win32Message();
                return false;
            }

            if (SetClipboardData(CfUnicodeText, memory) == IntPtr.Zero)
            {
                error = Win32Message();
                return false;
            }

            // The clipboard now owns the global memory.
            memory = IntPtr.Zero;
        }
        finally
        {
            CloseClipboard();
            if (memory != IntPtr.Zero)
            {
                GlobalFree(memory);
            }
        }

        return true;
    }

    public static bool TryGetText(out byte[] utf8, out string error)
    {
        utf8 = Array.Empty<byte>();
        error = string.Empty;

        if (!TryOpen(out error))
        {
            return false;
        }

        try
        {
            if (IsClipboardFormatAvailable(CfUnicodeText) == 0)
            {
                error = "the clipboard does not contain text";
                return false;
            }

            IntPtr handle = GetClipboardData(CfUnicodeText);
            if (handle == IntPtr.Zero)
            {
                error = Win32Message();
                return false;
            }

            IntPtr pointer = GlobalLock(handle);
            if (pointer == IntPtr.Zero)
            {
                error = Win32Message();
                return false;
            }

            try
            {
                string text = Marshal.PtrToStringUni(pointer) ?? string.Empty;
                utf8 = Encoding.UTF8.GetBytes(text);
            }
            finally
            {
                GlobalUnlock(handle);
            }

            return true;
        }
        finally
        {
            CloseClipboard();
        }
    }

    private static bool TryOpen(out string error)
    {
        error = string.Empty;
        int code = 0;
        for (int attempt = 0; attempt < OpenAttempts; ++attempt)
        {
            if (OpenClipboard(IntPtr.Zero) != 0)
            {
                return true;
            }

            code = Marshal.GetLastPInvokeError();
            Thread.Sleep(OpenRetryDelayMilliseconds);
        }

        error = Win32Message(code);
        return false;
    }

    private static string Win32Message()
        => Win32Message(Marshal.GetLastPInvokeError());

    private static string Win32Message(int code)
        => new Win32Exception(code).Message;
}

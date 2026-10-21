namespace Hexal.Util;

/// <summary>
/// Binary file I/O with atomic replacement. Saving writes a sibling temporary
/// file and renames it over the target, so a failed write never truncates the
/// original document.
/// </summary>
public static class FileSystemUtil
{
    private static int s_tempCounter;

    public static bool ReadFile(string path, out byte[] contents, out string error)
    {
        contents = Array.Empty<byte>();
        error = string.Empty;

        if (string.IsNullOrEmpty(path))
        {
            error = "empty file name";
            return false;
        }

        try
        {
            if (Directory.Exists(path))
            {
                error = $"'{path}' is a directory";
                return false;
            }

            contents = File.ReadAllBytes(path);
            return true;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            error = $"cannot read '{path}': {ex.Message}";
            return false;
        }
    }

    public static bool AtomicWrite(string target, Action<Stream> writer, out string error)
    {
        error = string.Empty;

        if (string.IsNullOrEmpty(target))
        {
            error = "empty file name";
            return false;
        }

        string parent = Path.GetDirectoryName(target) ?? string.Empty;
        if (parent.Length == 0)
        {
            parent = ".";
        }

        if (!Directory.Exists(parent))
        {
            error = $"directory does not exist: '{parent}'";
            return false;
        }

        if (Directory.Exists(target))
        {
            error = $"'{target}' is a directory";
            return false;
        }

        string baseName = Path.GetFileName(target);
        if (baseName.Length == 0)
        {
            baseName = "hexal";
        }

        FileStream? stream = null;
        string temp = string.Empty;

        for (int attempt = 0; attempt < 100; ++attempt)
        {
            int suffixNumber = Interlocked.Increment(ref s_tempCounter);
            temp = Path.Combine(parent,
                $"{baseName}.hexal-tmp-{Environment.ProcessId}-{suffixNumber}");

            try
            {
                stream = new FileStream(temp, FileMode.CreateNew, FileAccess.Write,
                    FileShare.None, 64 * 1024, FileOptions.SequentialScan);
                break;
            }
            catch (IOException) when (File.Exists(temp))
            {
                // Another process chose the same name; try the next suffix.
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                       NotSupportedException or ArgumentException)
            {
                error = $"cannot create temporary file in '{parent}': {ex.Message}";
                return false;
            }
        }

        if (stream is null)
        {
            error = $"cannot create a unique temporary file in '{parent}'";
            return false;
        }

        try
        {
            using (stream)
            {
                writer(stream);
                // Match the C++ MVP: flush to the OS, then rename atomically.
                // A power-loss fsync is intentionally not part of the MVP.
                stream.Flush();
            }
        }
        catch (Exception ex)
        {
            DeleteQuietly(temp);
            error = $"write failed for '{target}': {ex.Message}";
            return false;
        }

        PreservePermissions(target, temp);

        try
        {
            // Same-directory rename: atomic on POSIX, MoveFileEx with replace on
            // Windows. The target is never truncated in place.
            File.Move(temp, target, overwrite: true);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException or ArgumentException)
        {
            DeleteQuietly(temp);
            error = $"cannot replace '{target}': {ex.Message}";
            return false;
        }

        return true;
    }

    public static string ParentDirectory(string path)
    {
        string parent = Path.GetDirectoryName(path) ?? string.Empty;
        return parent.Length == 0 ? "." : parent;
    }

    private static void PreservePermissions(string target, string temp)
    {
        if (OperatingSystem.IsWindows())
        {
            return;
        }

        try
        {
            UnixFileMode mode = File.GetUnixFileMode(target);
            File.SetUnixFileMode(temp, mode);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or
                                   NotSupportedException)
        {
            // Preserving permissions is best effort, matching the C++ MVP.
        }
    }

    private static void DeleteQuietly(string path)
    {
        try
        {
            File.Delete(path);
        }
        catch
        {
            // Best effort cleanup of the temporary file.
        }
    }
}

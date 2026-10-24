using System.Diagnostics;
using System.Text;

namespace Hexal.Util;

/// <summary>
/// Reads the workspace's Git status and reports files that are modified,
/// renamed, copied or type-changed relative to HEAD. The sidebar uses this to
/// show the familiar "M" marker on files and their parent folders.
/// </summary>
public static class GitStatus
{
    private const int HelperTimeoutMilliseconds = 3000;

    public static HashSet<string> LoadModifiedPaths(string workspaceRoot)
    {
        var result = new HashSet<string>(
            OperatingSystem.IsWindows() ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal
        );

        string? repositoryRoot = RunGit(workspaceRoot, "rev-parse", "--show-toplevel")?.Trim();
        if (string.IsNullOrEmpty(repositoryRoot))
        {
            return result;
        }

        string? status = RunGit(
            workspaceRoot,
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=no"
        );
        if (string.IsNullOrEmpty(status))
        {
            return result;
        }

        int position = 0;
        while (position + 3 <= status.Length)
        {
            char indexStatus = status[position];
            char workTreeStatus = status[position + 1];
            position += 3;

            int end = status.IndexOf('\0', position);
            if (end < 0)
            {
                break;
            }

            string path = status[position..end];
            position = end + 1;

            bool renamedOrCopied =
                indexStatus is 'R' or 'C' || workTreeStatus is 'R' or 'C';
            if (renamedOrCopied)
            {
                int oldEnd = status.IndexOf('\0', position);
                if (oldEnd < 0)
                {
                    break;
                }
                position = oldEnd + 1;
            }

            bool modified =
                indexStatus is 'M' or 'T' or 'R' or 'C'
                || workTreeStatus is 'M' or 'T' or 'R' or 'C';
            if (!modified)
            {
                continue;
            }

            try
            {
                string relative = path.Replace('/', Path.DirectorySeparatorChar);
                result.Add(Path.GetFullPath(Path.Combine(repositoryRoot, relative)));
            }
            catch (Exception ex)
                when (ex is ArgumentException or NotSupportedException or PathTooLongException) { }
        }

        return result;
    }

    private static string? RunGit(string directory, params string[] arguments)
    {
        try
        {
            var startInfo = new ProcessStartInfo
            {
                FileName = "git",
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                StandardOutputEncoding = Encoding.UTF8,
                StandardErrorEncoding = Encoding.UTF8,
            };

            startInfo.ArgumentList.Add("-C");
            startInfo.ArgumentList.Add(directory);
            foreach (string argument in arguments)
            {
                startInfo.ArgumentList.Add(argument);
            }

            using Process? process = Process.Start(startInfo);
            if (process is null)
            {
                return null;
            }

            process.StandardInput.Close();
            Task<string> outputTask = process.StandardOutput.ReadToEndAsync();
            Task<string> errorTask = process.StandardError.ReadToEndAsync();

            if (!process.WaitForExit(HelperTimeoutMilliseconds))
            {
                try
                {
                    process.Kill(entireProcessTree: true);
                }
                catch (Exception ex)
                    when (ex
                            is InvalidOperationException
                                or NotSupportedException
                                or System.ComponentModel.Win32Exception
                    )
                {
                    // The process may already have exited.
                }

                return null;
            }

            string output = outputTask.GetAwaiter().GetResult();
            _ = errorTask.GetAwaiter().GetResult();
            return process.ExitCode == 0 ? output : null;
        }
        catch (Exception ex)
            when (ex
                    is System.ComponentModel.Win32Exception
                        or IOException
                        or InvalidOperationException
                        or NotSupportedException
                        or UnauthorizedAccessException
            )
        {
            return null;
        }
    }
}

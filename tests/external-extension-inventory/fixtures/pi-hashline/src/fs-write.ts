import { randomUUID } from "node:crypto";
import { chmod, lstat, mkdir, open, readlink, rename, stat, unlink } from "node:fs/promises";
import { dirname, join, parse, resolve, sep } from "node:path";
import { type FileSnapshot, getFileSnapshot, sameFileSnapshot } from "./snapshot";

export async function resolveMutationTargetPath(path: string): Promise<string> {
  const absolutePath = resolve(path);
  const { root } = parse(absolutePath);
  const parts = absolutePath.slice(root.length).split(sep).filter((part) => part.length > 0);
  const visitedSymlinks = new Set<string>();

  async function resolveFromParts(currentPath: string, remainingParts: string[]): Promise<string> {
    if (remainingParts.length === 0) return currentPath;

    const [nextPart, ...tail] = remainingParts;
    const candidatePath = join(currentPath, nextPart!);

    try {
      const candidateStats = await lstat(candidatePath);
      if (!candidateStats.isSymbolicLink()) {
        return resolveFromParts(candidatePath, tail);
      }

      if (visitedSymlinks.has(candidatePath)) {
        const error = new Error(`Too many symbolic links while resolving ${path}`) as NodeJS.ErrnoException;
        error.code = "ELOOP";
        throw error;
      }
      visitedSymlinks.add(candidatePath);

      const linkTargetPath = resolve(dirname(candidatePath), await readlink(candidatePath));
      const targetRoot = parse(linkTargetPath).root;
      const targetParts = linkTargetPath
        .slice(targetRoot.length)
        .split(sep)
        .filter((part) => part.length > 0);
      return resolveFromParts(targetRoot, [...targetParts, ...tail]);
    } catch (error: unknown) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") {
        return join(candidatePath, ...tail);
      }
      throw error;
    }
  }

  return resolveFromParts(root, parts);
}

async function syncDirectory(path: string): Promise<void> {
  let dirHandle: Awaited<ReturnType<typeof open>> | undefined;
  try {
    dirHandle = await open(path, "r");
    await dirHandle.sync();
  } catch (error: unknown) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code !== "EINVAL" && code !== "ENOTSUP" && code !== "EISDIR") throw error;
  } finally {
    await dirHandle?.close();
  }
}

async function writeTempFileDurably(tempPath: string, content: string, mode: number): Promise<void> {
  const fileMode = mode & 0o7777;
  const fileHandle = await open(tempPath, "wx", fileMode);
  try {
    await fileHandle.writeFile(content, "utf-8");
    await chmod(tempPath, fileMode);
    await fileHandle.sync();
  } finally {
    await fileHandle.close();
  }
}

function assertNotHardlinked(filePath: string, linkCount: number): void {
  if (linkCount > 1) {
    throw new Error(`[E_HARDLINK_UNSUPPORTED] Refusing to edit hardlinked file: ${filePath}. Atomic replacement would break other hardlinks; copy the file to a non-hardlinked path and retry.`);
  }
}

export async function writeTextFileAtomically(
  path: string,
  content: string,
  options: { expectedSnapshot?: FileSnapshot } = {},
): Promise<FileSnapshot> {
  const targetPath = await resolveMutationTargetPath(path);
  const currentStat = await stat(targetPath);
  assertNotHardlinked(targetPath, currentStat.nlink);
  const dir = dirname(targetPath);
  const tempPath = join(dir, `.pi-hashline-${randomUUID()}.tmp`);
  let renamed = false;

  try {
    await mkdir(dir, { recursive: true });
    await writeTempFileDurably(tempPath, content, currentStat.mode);

    if (options.expectedSnapshot) {
      const latestSnapshot = await getFileSnapshot(targetPath);
      if (!sameFileSnapshot(options.expectedSnapshot, latestSnapshot)) {
        throw new Error("[E_CONCURRENT_MODIFICATION] File changed while edit was being prepared. Re-read and retry with fresh anchors.");
      }
    }

    const latestStat = await stat(targetPath);
    assertNotHardlinked(targetPath, latestStat.nlink);

    await rename(tempPath, targetPath);
    renamed = true;
    await syncDirectory(dir);
    return getFileSnapshot(targetPath);
  } finally {
    if (!renamed) {
      await unlink(tempPath).catch((error: unknown) => {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      });
    }
  }
}

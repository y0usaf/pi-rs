import { stat } from "node:fs/promises";

export type FileSnapshot = {
  dev: number;
  ino: number;
  size: number;
  mtimeMs: number;
  ctimeMs: number;
  snapshotId: string;
};

export async function getFileSnapshot(path: string): Promise<FileSnapshot> {
  const stats = await stat(path);
  const snapshot = {
    dev: stats.dev,
    ino: stats.ino,
    size: stats.size,
    mtimeMs: stats.mtimeMs,
    ctimeMs: stats.ctimeMs,
  };
  return {
    ...snapshot,
    snapshotId: `${snapshot.dev}:${snapshot.ino}:${snapshot.size}:${snapshot.mtimeMs}:${snapshot.ctimeMs}`,
  };
}

export function sameFileSnapshot(left: FileSnapshot, right: FileSnapshot): boolean {
  return left.dev === right.dev &&
    left.ino === right.ino &&
    left.size === right.size &&
    left.mtimeMs === right.mtimeMs &&
    left.ctimeMs === right.ctimeMs;
}

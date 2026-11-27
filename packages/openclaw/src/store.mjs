import { constants } from "node:fs";
import { chmod, lstat, mkdir, open, rename, unlink } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import path from "node:path";

const fields = new Set([
  "version", "sessionKey", "agentId", "sessionId", "cwd", "mode", "binding",
  "generation",
]);

function nonempty(value, name) {
  if (typeof value !== "string" || value.length === 0) {
    throw new TypeError(`${name} must be a nonempty string`);
  }
}

export function ownerKey(input) {
  if (!input || typeof input !== "object") {
    throw new TypeError("session owner must be an object");
  }
  nonempty(input.sessionKey, "sessionKey");
  if (input.agentId !== undefined) nonempty(input.agentId, "agentId");
  return JSON.stringify([input.agentId ?? "", input.sessionKey]);
}

function validate(key, record) {
  if (!record || typeof record !== "object" || Array.isArray(record)) {
    throw new TypeError("session record must be an object");
  }
  for (const field of Object.keys(record)) {
    if (!fields.has(field)) throw new TypeError(`unknown session field: ${field}`);
  }
  if (record.version !== 1) throw new TypeError("unsupported session record version");
  for (const field of ["sessionKey", "sessionId", "generation"]) {
    nonempty(record[field], field);
  }
  if (record.agentId !== undefined) nonempty(record.agentId, "agentId");
  if (typeof record.cwd !== "string" || !path.isAbsolute(record.cwd)) {
    throw new TypeError("cwd must be an absolute path");
  }
  if (record.mode !== "persistent" && record.mode !== "oneshot") {
    throw new TypeError("invalid session mode");
  }
  if (typeof record.binding !== "string" || !/^[a-f0-9]{64}$/.test(record.binding)) {
    throw new TypeError("binding must be a SHA256 fingerprint");
  }
  if (ownerKey(record) !== key) throw new TypeError("session record owner mismatch");
}

async function regularFile(filename) {
  try {
    const stat = await lstat(filename);
    if (!stat.isFile() || stat.isSymbolicLink()) {
      throw new Error("session file must be a regular file, not a symlink");
    }
    return stat;
  } catch (error) {
    if (error.code === "ENOENT") return undefined;
    throw error;
  }
}

export class SessionStore {
  constructor(stateDir) {
    nonempty(stateDir, "stateDir");
    this.stateDir = path.resolve(stateDir);
  }

  filename(key) {
    if (typeof key !== "string") throw new TypeError("owner key must be a string");
    return path.join(this.stateDir, `${createHash("sha256").update(key).digest("hex")}.json`);
  }

  async directory(create = false) {
    if (create) await mkdir(this.stateDir, { recursive: true, mode: 0o700 });
    try {
      const stat = await lstat(this.stateDir);
      if (!stat.isDirectory() || stat.isSymbolicLink()) {
        throw new Error("session store must be a directory, not a symlink");
      }
      if (create) await chmod(this.stateDir, 0o700);
      return true;
    } catch (error) {
      if (!create && error.code === "ENOENT") return false;
      throw error;
    }
  }

  async read(key) {
    const filename = this.filename(key);
    if (!await this.directory()) return undefined;
    if (!await regularFile(filename)) return undefined;
    let handle;
    try {
      handle = await open(filename, constants.O_RDONLY | constants.O_NOFOLLOW);
      const stat = await handle.stat();
      if (!stat.isFile()) throw new Error("session file must be a regular file");
      const record = JSON.parse(await handle.readFile("utf8"));
      validate(key, record);
      return record;
    } catch (error) {
      if (error.code === "ENOENT") return undefined;
      throw error;
    } finally {
      await handle?.close();
    }
  }

  async write(key, record) {
    const filename = this.filename(key);
    validate(key, record);
    // Serialize before awaiting so callers cannot mutate the validated record.
    const contents = `${JSON.stringify(record)}\n`;
    await this.directory(true);
    await regularFile(filename);
    const temporary = path.join(this.stateDir, `.${randomUUID()}.tmp`);
    let handle;
    try {
      handle = await open(temporary,
        constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
        0o600);
      await handle.writeFile(contents, "utf8");
      await handle.sync();
      await handle.close();
      handle = undefined;
      await regularFile(filename);
      await rename(temporary, filename);
    } finally {
      await handle?.close();
      await unlink(temporary).catch((error) => {
        if (error.code !== "ENOENT") throw error;
      });
    }
  }

  async delete(key) {
    const filename = this.filename(key);
    if (!await this.directory()) return;
    if (!await regularFile(filename)) return;
    await unlink(filename).catch((error) => {
      if (error.code !== "ENOENT") throw error;
    });
  }
}

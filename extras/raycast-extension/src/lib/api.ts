// The daemon's API: NDJSON on its Unix socket, `state/daemon.sock`
// ($XDG_STATE_HOME/aleph, else ~/.local/state/aleph; daemon.rs `sock_path`).
// One `{id, method, params}` line out, one `{id, result}` or
// `{id, error: {code, message}}` line back, one request per connection. The
// socket is this user's own (0600), so it has every scope. The methods are
// api.rs's `METHODS` (`aleph api methods`).
//
// No Raycast imports: plain node tests it (model.test.ts runs a fake daemon).
// Callers fall back to the CLI's `--json` when there's no daemon or it fails
// (`apiOr`), so a missing socket costs one failed connect, and a daemon that
// hangs is skipped for a while after one timeout.
import { createConnection } from "node:net";
import { homedir } from "node:os";
import { join } from "node:path";

/** How long one request may take before the CLI answers instead. */
export const API_TIMEOUT_MS = 2500;
/** After a timeout, don't wait on the daemon again for this long (this command's run). */
const SKIP_AFTER_TIMEOUT_MS = 30_000;
/** A daemon over the network gets longer than the local socket. */
const REMOTE_TIMEOUT_MS = 6000;

export class ApiError extends Error {
  constructor(
    message: string,
    /** `unavailable` (no socket, refused, closed), `timeout`, `parse`, or the daemon's own (`unknown`, `invalid`, `failed`, `forbidden`, …) */
    public code: string,
  ) {
    super(message);
  }
}

type Env = Record<string, string | undefined>;

/** `$XDG_STATE_HOME/aleph`, else `~/.local/state/aleph` (config.rs `state_dir`). */
export function stateDir(env: Env = process.env, home = homedir()): string {
  const x = env.XDG_STATE_HOME?.trim();
  return join(x || join(home, ".local", "state"), "aleph");
}

/**
 * Where the daemon's socket is: `<state>/daemon.sock`, or — when that path is
 * 100 bytes or longer (Unix socket paths are short) — `/tmp/al-<uid>/d-<hash>.sock`,
 * the hash being the low 32 bits of FNV-1a over the long path's bytes (daemon.rs).
 */
export function socketPath(env: Env = process.env, home = homedir(), uid = process.getuid?.() ?? 0): string {
  const p = join(stateDir(env, home), "daemon.sock");
  const bytes = Buffer.from(p, "utf8");
  if (bytes.length < 100) return p;
  let x = 0xcbf29ce484222325n;
  for (const b of bytes) x = BigInt.asUintN(64, (x ^ BigInt(b)) * 0x100000001b3n);
  const h = Number(x & 0xffffffffn)
    .toString(16)
    .padStart(8, "0");
  return `/tmp/al-${uid}/d-${h}.sock`;
}

let seq = 0;
let skipUntil = 0;

/** A daemon on another computer, over its HTTP API (the Daemon URL and token preferences). */
let remote: { url: string; token: string } | undefined;

/** Talk to the daemon at `url` (with a device token) instead of this computer's socket; empty: the socket. */
export function useRemote(url?: string, token?: string): void {
  const u = url?.trim().replace(/\/+$/, "");
  remote = u ? { url: u, token: token?.trim() ?? "" } : undefined;
}

/** `POST /api/v1/<method>`: `{"result": …}` or `{"error": {code, message}}`. */
async function overHttp<T>(method: string, params: object, timeoutMs: number): Promise<T> {
  const r0 = remote as { url: string; token: string };
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const res = await fetch(`${r0.url}/api/v1/${method}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Aleph": "1",
        ...(r0.token ? { Authorization: `Bearer ${r0.token}` } : {}),
      },
      body: JSON.stringify(params),
      signal: ctl.signal,
    });
    const v = (await res.json().catch(() => ({}))) as { result?: T; error?: { code?: string; message?: string } };
    if (v.error) throw new ApiError(v.error.message ?? `HTTP ${res.status}`, v.error.code ?? "failed");
    if (!res.ok) throw new ApiError(`the daemon at ${r0.url} answered HTTP ${res.status}`, "failed");
    return v.result as T;
  } catch (e) {
    if (e instanceof ApiError) throw e;
    throw ctl.signal.aborted
      ? new ApiError(`the daemon at ${r0.url} didn't answer ${method} in time`, "timeout")
      : new ApiError(`the daemon at ${r0.url}: ${e instanceof Error ? e.message : String(e)}`, "unavailable");
  } finally {
    clearTimeout(timer);
  }
}

export interface RequestOptions {
  timeoutMs?: number;
  /** the socket (default: `socketPath()`) */
  path?: string;
}

/** One request to the daemon; its `result`. Rejects with an `ApiError`. */
export function request<T>(method: string, params: object = {}, o: RequestOptions = {}): Promise<T> {
  if (!o.path && remote) return overHttp<T>(method, params, o.timeoutMs ?? REMOTE_TIMEOUT_MS);
  if (!o.path && Date.now() < skipUntil) {
    return Promise.reject(new ApiError("the daemon was slow to answer a moment ago", "timeout"));
  }
  const path = o.path ?? socketPath();
  const id = `raycast-${process.pid}-${++seq}`;
  return new Promise<T>((resolve, reject) => {
    let buf = "";
    let done = false;
    const sock = createConnection(path);
    const finish = (err: ApiError | undefined, value?: T) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      sock.destroy();
      if (err) reject(err);
      else resolve(value as T);
    };
    const timer = setTimeout(() => {
      if (!o.path) skipUntil = Date.now() + SKIP_AFTER_TIMEOUT_MS;
      finish(new ApiError(`the daemon didn't answer ${method} in time`, "timeout"));
    }, o.timeoutMs ?? API_TIMEOUT_MS);
    sock.setEncoding("utf8");
    sock.on("connect", () => sock.write(`${JSON.stringify({ id, method, params })}\n`));
    sock.on("data", (chunk: string) => {
      buf += chunk;
      for (let i = buf.indexOf("\n"); i >= 0; i = buf.indexOf("\n")) {
        const line = buf.slice(0, i).trim();
        buf = buf.slice(i + 1);
        if (!line) continue;
        let v: { id?: unknown; result?: T; error?: { code?: string; message?: string } };
        try {
          v = JSON.parse(line);
        } catch {
          return finish(new ApiError("the daemon's answer isn't JSON", "parse"));
        }
        // `id: null` is the daemon refusing before it read us (busy, too long)
        if (v.id !== id && v.id !== null) continue;
        if (v.error) return finish(new ApiError(v.error.message ?? "the daemon refused it", v.error.code ?? "failed"));
        return finish(undefined, v.result as T);
      }
    });
    sock.on("error", (e: NodeJS.ErrnoException) =>
      finish(
        new ApiError(
          e.code === "ENOENT" || e.code === "ECONNREFUSED"
            ? "no daemon is running"
            : `the daemon's socket: ${e.message}`,
          "unavailable",
        ),
      ),
    );
    sock.on("close", () => finish(new ApiError("the daemon closed the connection without answering", "unavailable")));
  });
}

/** One API call: the method, its params, and whether an answer is good enough to use. */
export interface ApiCall<T> {
  method: string;
  params?: object;
  /** e.g. a snapshot from a daemon that hasn't polled yet isn't */
  accept?: (v: T) => boolean;
  /** how long this one may take (default `API_TIMEOUT_MS`) */
  timeoutMs?: number;
}

/**
 * The daemon's answer, or — when there's no daemon, it errs, or the answer
 * isn't accepted — the fallback (the CLI's `--json`).
 */
export async function apiOr<T>(call: ApiCall<T>, fallback: () => Promise<T>): Promise<T> {
  try {
    const v = await request<T>(call.method, call.params ?? {}, { timeoutMs: call.timeoutMs });
    if (!call.accept || call.accept(v)) return v;
  } catch {
    // no daemon (or an older one without this method): the CLI answers
  }
  return fallback();
}

/**
 * Something done (send, approve, close…) through the daemon; the CLI does it
 * only when there's no daemon or it doesn't know the method — never after a
 * timeout or a refusal, so nothing is done twice. The daemon's own errors
 * (no such session, …) come back as they are.
 */
export async function act<T>(
  method: string,
  params: object,
  fallback: () => Promise<T>,
  timeoutMs = 30_000,
): Promise<T> {
  try {
    return await request<T>(method, params, { timeoutMs });
  } catch (e) {
    if (e instanceof ApiError && (e.code === "unavailable" || e.code === "unknown")) return fallback();
    throw e;
  }
}

/** Is a daemon (or a dashboard serving while none runs) answering on the socket? */
export async function daemonAnswers(): Promise<boolean> {
  try {
    await request("daemon.ping", {}, { timeoutMs: 1500 });
    return true;
  } catch {
    return false;
  }
}

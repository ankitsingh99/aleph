// `npm test`: the daemon client (api.ts) against a fake daemon on a Unix socket.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer, Server } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { ApiError, apiOr, request, socketPath, stateDir } from "./api";

const dir = mkdtempSync(join(tmpdir(), "aleph-api-"));
after(() => rmSync(dir, { recursive: true, force: true }));

/** A daemon that answers each request line with `answer(request)` (undefined: never answers). */
function fakeDaemon(
  name: string,
  answer: (req: { id: string; method: string; params: unknown }) => unknown,
): Promise<{ path: string; server: Server }> {
  const path = join(dir, name);
  const server = createServer((c) => {
    let buf = "";
    c.setEncoding("utf8");
    c.on("data", (d: string) => {
      buf += d;
      const i = buf.indexOf("\n");
      if (i < 0) return;
      const req = JSON.parse(buf.slice(0, i));
      const out = answer(req);
      if (out === undefined) return;
      for (const line of Array.isArray(out) ? out : [out]) c.write(`${JSON.stringify(line)}\n`);
    });
  });
  return new Promise((resolve) => server.listen(path, () => resolve({ path, server })));
}

test("where the socket is (daemon.rs sock_path)", () => {
  assert.equal(stateDir({}, "/Users/me"), "/Users/me/.local/state/aleph");
  assert.equal(stateDir({ XDG_STATE_HOME: "/x/state" }, "/Users/me"), "/x/state/aleph");
  assert.equal(socketPath({}, "/Users/me"), "/Users/me/.local/state/aleph/daemon.sock");
  // 100 bytes or more: a short per-user path, the same FNV-1a hash the daemon uses
  const long =
    "/Users/me/work/projects/aleph/.claude/worktrees/agent-0123456789abcdef0/tests/e2e-fake/.run/xstate";
  assert.equal(socketPath({ XDG_STATE_HOME: long }, "/Users/me", 501), "/tmp/al-501/d-6e8fafcf.sock");
});

test("a request and its answer", async () => {
  const { path, server } = await fakeDaemon("ok.sock", (r) => [
    // a line for someone else first (a shared connection): skipped
    { id: "other", result: { no: true } },
    { id: r.id, result: { method: r.method, params: r.params } },
  ]);
  try {
    const v = await request<{ method: string; params: { n: number } }>("inbox.list", { n: 3 }, { path });
    assert.deepEqual(v, { method: "inbox.list", params: { n: 3 } });
  } finally {
    server.close();
  }
});

test("the daemon's errors keep their code", async () => {
  const { path, server } = await fakeDaemon("err.sock", (r) =>
    r.method === "busy"
      ? { id: null, error: { code: "busy", message: "too many connections" } }
      : { id: r.id, error: { code: "unknown", message: `no such method: ${r.method}` } },
  );
  try {
    await assert.rejects(
      request("nope", {}, { path }),
      (e: ApiError) => e.code === "unknown" && /nope/.test(e.message),
    );
    await assert.rejects(request("busy", {}, { path }), (e: ApiError) => e.code === "busy");
  } finally {
    server.close();
  }
});

test("no daemon, or one that doesn't answer", async () => {
  await assert.rejects(
    request("daemon.ping", {}, { path: join(dir, "none.sock") }),
    (e: ApiError) => e.code === "unavailable",
  );
  const { path, server } = await fakeDaemon("mute.sock", () => undefined);
  try {
    const t0 = Date.now();
    await assert.rejects(request("daemon.ping", {}, { path, timeoutMs: 150 }), (e: ApiError) => e.code === "timeout");
    assert.ok(Date.now() - t0 < 2000);
  } finally {
    server.close();
  }
});

test("the CLI answers when the daemon can't", async () => {
  // the real socket path of this test run: nothing listens there
  const saved = process.env.XDG_STATE_HOME;
  process.env.XDG_STATE_HOME = join(dir, "st");
  try {
    assert.equal(await apiOr({ method: "state.snapshot" }, async () => "from the cli"), "from the cli");
  } finally {
    if (saved === undefined) delete process.env.XDG_STATE_HOME;
    else process.env.XDG_STATE_HOME = saved;
  }
});

test("a daemon on another computer: over HTTP, with the token and the CSRF header", async () => {
  const { createServer: createHttp } = await import("node:http");
  let seen: { url?: string; auth?: string; csrf?: string; body?: string } = {};
  const srv = createHttp((req, res) => {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      seen = { url: req.url, auth: req.headers.authorization, csrf: req.headers["x-aleph"] as string, body };
      res.setHeader("Content-Type", "application/json");
      if (req.url === "/api/v1/inbox.read") {
        res.statusCode = 403;
        res.end(JSON.stringify({ error: { code: "scope", message: "this token can only read" } }));
        return;
      }
      res.end(JSON.stringify({ result: { gen: 7 } }));
    });
  });
  await new Promise<void>((r) => srv.listen(0, "127.0.0.1", () => r()));
  const port = (srv.address() as { port: number }).port;
  const { useRemote } = await import("./api");
  useRemote(`http://127.0.0.1:${port}/`, "tok123");
  try {
    const v = await request<{ gen: number }>("state.snapshot", { a: 1 });
    assert.equal(v.gen, 7);
    assert.equal(seen.url, "/api/v1/state.snapshot");
    assert.equal(seen.auth, "Bearer tok123");
    assert.equal(seen.csrf, "1");
    assert.equal(seen.body, '{"a":1}');
    await assert.rejects(request("inbox.read", {}), (e: ApiError) => e.code === "scope" && /only read/.test(e.message));
  } finally {
    useRemote(undefined);
    srv.close();
  }
});

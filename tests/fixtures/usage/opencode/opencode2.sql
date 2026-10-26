-- A synthetic OpenCode 2 opencode.db: sessions in session_v2, their messages in
-- session_message (one row per user prompt, request, prompt end, compaction).
-- An OpenCode 1 message copied over keeps its id: the old table still holds it,
-- and the reader must take the new tables alone. Built by the tests with
-- `sqlite3 opencode.db < opencode2.sql` when sqlite3 is installed.
CREATE TABLE `message` (
  `id` text PRIMARY KEY,
  `session_id` text NOT NULL,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL,
  `data` text NOT NULL
);
CREATE TABLE `session_v2` (
  `id` text PRIMARY KEY,
  `project_id` text NOT NULL,
  `parent_id` text,
  `slug` text NOT NULL,
  `directory` text NOT NULL,
  `title` text,
  `version` text NOT NULL,
  `agent` text,
  `model` text,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL,
  `time_idle` integer,
  `idle_outcome` text
);
CREATE TABLE `session_message` (
  `id` text PRIMARY KEY,
  `session_id` text NOT NULL,
  `type` text NOT NULL,
  `seq` integer NOT NULL,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL,
  `data` text NOT NULL
);
INSERT INTO message VALUES ('msg_v2a2', 'ses_v2a', 1792661001000, 1792661009000,
  '{"role":"assistant","modelID":"gpt-5.1-codex","providerID":"openai","path":{"cwd":"/home/u/app"},"tokens":{"input":1,"output":1,"reasoning":0,"cache":{"read":0,"write":0}},"cost":0}');

INSERT INTO session_v2 VALUES ('ses_v2a', 'prj_2', NULL, 'calm-river', '/home/u/app', 'SECRET title', '2.1.2', 'build',
  '{"id":"gpt-5.1-codex","providerID":"openai","variant":"high"}', 1792661000000, 1792661200000, 1792661200000, 'interrupted');
INSERT INTO session_v2 VALUES ('ses_v2b', 'prj_2', 'ses_v2a', 'calm-river-explore', '/home/u/app', 'SECRET child', '2.1.2', 'explore',
  '{"id":"gpt-5.1-codex","providerID":"openai","variant":"high"}', 1792661003000, 1792661006000, NULL, NULL);

-- prompt 1: two requests (a read, a subagent), then the prompt ends 30 s after it began
INSERT INTO session_message VALUES ('msg_v2a1', 'ses_v2a', 'user', 1, 1792661000000, 1792661000000,
  '{"time":{"created":1792661000000},"text":"SECRET prompt","files":[],"agents":[]}');
INSERT INTO session_message VALUES ('msg_v2a2', 'ses_v2a', 'assistant', 2, 1792661001000, 1792661009000,
  '{"time":{"created":1792661001000,"completed":1792661009000},"agent":"build","model":{"id":"gpt-5.1-codex","providerID":"openai","variant":"high"},"finish":"tool-calls","cost":0,"tokens":{"input":900,"output":120,"reasoning":40,"cache":{"read":6000,"write":0}},"content":[{"type":"reasoning","text":"SECRET thought"},{"type":"tool","id":"call_r1","name":"read","state":{"status":"completed","input":{"filePath":"/home/u/app/SECRET.rs"}},"time":{"created":1792661002000,"completed":1792661002500}},{"type":"tool","id":"call_t1","name":"task","state":{"status":"completed","input":{"subagent_type":"explore","prompt":"SECRET task"},"metadata":{"sessionId":"ses_v2b","parentSessionId":"ses_v2a"}},"time":{"created":1792661003000,"completed":1792661008000}}]}');
INSERT INTO session_message VALUES ('msg_v2a3', 'ses_v2a', 'assistant', 3, 1792661010000, 1792661029000,
  '{"time":{"created":1792661010000,"completed":1792661029000},"agent":"build","model":{"id":"gpt-5.1-codex","providerID":"openai","variant":"high"},"finish":"stop","cost":0.02,"tokens":{"input":300,"output":500,"reasoning":100,"cache":{"read":7000,"write":0}},"content":[{"type":"text","text":"SECRET answer"}]}');
INSERT INTO session_message VALUES ('msg_v2a4', 'ses_v2a', 'idle', 4, 1792661030000, 1792661030000,
  '{"time":{"created":1792661030000},"outcome":"succeeded"}');

-- the subagent: its own session, prompted by the agent
INSERT INTO session_message VALUES ('msg_v2b1', 'ses_v2b', 'user', 1, 1792661003500, 1792661003500,
  '{"time":{"created":1792661003500},"text":"SECRET task"}');
INSERT INTO session_message VALUES ('msg_v2b2', 'ses_v2b', 'assistant', 2, 1792661004000, 1792661006000,
  '{"time":{"created":1792661004000,"completed":1792661006000},"agent":"explore","model":{"id":"gpt-5.1-codex","providerID":"openai","variant":"high"},"finish":"stop","cost":0,"tokens":{"input":2000,"output":80,"reasoning":10,"cache":{"read":0,"write":0}},"content":[{"type":"tool","id":"call_g1","name":"grep","state":{"status":"error","input":{"pattern":"SECRET"}},"time":{"created":1792661004500,"completed":1792661004600}}]}');

-- prompt 2, compacted, carried on by OpenCode itself, then interrupted
INSERT INTO session_message VALUES ('msg_v2a5', 'ses_v2a', 'user', 5, 1792661100000, 1792661100000,
  '{"time":{"created":1792661100000},"text":"SECRET second"}');
INSERT INTO session_message VALUES ('msg_v2a6', 'ses_v2a', 'compaction', 6, 1792661110000, 1792661120000,
  '{"time":{"created":1792661110000},"reason":"auto","status":"completed","summary":"SECRET summary"}');
INSERT INTO session_message VALUES ('msg_v2a7', 'ses_v2a', 'synthetic', 7, 1792661121000, 1792661121000,
  '{"time":{"created":1792661121000},"text":"SECRET continue"}');
INSERT INTO session_message VALUES ('msg_v2a8', 'ses_v2a', 'idle', 8, 1792661200000, 1792661200000,
  '{"time":{"created":1792661200000},"outcome":"interrupted"}');

-- OpenRouter through OpenCode (a paid model OpenCode priced, a free variant) and
-- one of OpenCode's own models, free
INSERT INTO session_v2 VALUES ('ses_v2c', 'prj_2', NULL, 'quiet-dune', '/home/u/app', 'SECRET', '2.1.2', 'build',
  '{"id":"anthropic/claude-sonnet-4.5","providerID":"openrouter"}', 1792661300000, 1792661400000, NULL, NULL);
INSERT INTO session_message VALUES ('msg_v2c1', 'ses_v2c', 'assistant', 1, 1792661301000, 1792661302000,
  '{"model":{"id":"anthropic/claude-sonnet-4.5","providerID":"openrouter"},"cost":0.0123,"tokens":{"input":1000,"output":100,"reasoning":0,"cache":{"read":0,"write":0}},"content":[]}');
INSERT INTO session_message VALUES ('msg_v2c2', 'ses_v2c', 'assistant', 2, 1792661303000, 1792661304000,
  '{"model":{"id":"qwen/qwen3-coder:free","providerID":"openrouter"},"cost":0,"tokens":{"input":2000,"output":200,"reasoning":0,"cache":{"read":0,"write":0}},"content":[]}');
INSERT INTO session_message VALUES ('msg_v2c3', 'ses_v2c', 'assistant', 3, 1792661305000, 1792661306000,
  '{"model":{"id":"big-pickle","providerID":"opencode"},"cost":0,"tokens":{"input":3000,"output":300,"reasoning":0,"cache":{"read":0,"write":0}},"content":[]}');

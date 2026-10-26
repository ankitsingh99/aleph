-- A synthetic opencode.db (its schema as of OpenCode's sqlite storage):
-- the tests build it with `sqlite3 opencode.db < opencode.sql` when sqlite3
-- is installed.
CREATE TABLE `session` (
  `id` text PRIMARY KEY,
  `project_id` text NOT NULL,
  `parent_id` text,
  `directory` text NOT NULL,
  `title` text NOT NULL,
  `cost` real DEFAULT 0 NOT NULL,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL
);
CREATE TABLE `part` (
  `id` text PRIMARY KEY,
  `message_id` text NOT NULL,
  `session_id` text NOT NULL,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL,
  `data` text NOT NULL
);
CREATE TABLE `message` (
  `id` text PRIMARY KEY,
  `session_id` text NOT NULL,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL,
  `data` text NOT NULL
);
INSERT INTO session VALUES ('ses_db01', 'prj_1', NULL, '/home/u/site', 'SECRET title', 0, 1792660000000, 1792660100000);
INSERT INTO message VALUES ('msg_db00', 'ses_db01', 1792659990000, 1792659990000,
  '{"role":"user","time":{"created":1792659990000},"agent":"build","model":{"providerID":"openai","modelID":"gpt-5.1-codex","variant":"xhigh"},"summary":{"title":"SECRET"}}');
INSERT INTO message VALUES ('msg_db01', 'ses_db01', 1792660000000, 1792660005000,
  '{"parentID":"msg_db00","role":"assistant","mode":"build","agent":"build","variant":"xhigh","path":{"cwd":"/home/u/site","root":"/home/u/site"},"cost":0,"tokens":{"total":20310,"input":12905,"output":613,"reasoning":136,"cache":{"write":0,"read":6656}},"modelID":"gpt-5.1-codex","providerID":"openai","time":{"created":1792660000000,"completed":1792660005000},"finish":"stop"}');
INSERT INTO message VALUES ('msg_db02', 'ses_db01', 1792660010000, 1792660020000,
  '{"parentID":"msg_db01","role":"assistant","mode":"build","agent":"build","variant":"xhigh","path":{"cwd":"/home/u/site","root":"/home/u/site"},"cost":0.0125,"tokens":{"total":9000,"input":1000,"output":500,"reasoning":0,"cache":{"write":2500,"read":5000}},"modelID":"claude-sonnet-4-5","providerID":"anthropic","time":{"created":1792660010000,"completed":1792660020000},"finish":"stop"}');

-- the timeline: tool calls (parts) and a subagent (a child session)
INSERT INTO session VALUES ('ses_db02', 'prj_1', 'ses_db01', '/home/u/site', 'SECRET child', 0, 1792660030000, 1792660040000);
INSERT INTO part VALUES ('prt_1', 'msg_db01', 'ses_db01', 1792660001000, 1792660004000,
  '{"type":"tool","tool":"bash","callID":"call_oc1","state":{"status":"completed","input":{"command":"SECRET"},"output":"SECRET","title":"SECRET","time":{"start":1792660001000,"end":1792660003500}}}');
INSERT INTO part VALUES ('prt_2', 'msg_db01', 'ses_db01', 1792660005000, 1792660006000,
  '{"type":"tool","tool":"task","callID":"call_oc2","state":{"status":"error","error":"SECRET","time":{"start":1792660005000,"end":1792660006000}}}');
INSERT INTO part VALUES ('prt_3', 'msg_db01', 'ses_db01', 1792660005000, 1792660006000,
  '{"type":"text","text":"SECRET text part"}');

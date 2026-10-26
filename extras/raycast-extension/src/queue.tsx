// Message Queue: every queued message across machines (`aleph queue ls
// --json`) — for one session (sent once it's done with its reply) or for the
// next idle agent on a machine. Send now, remove, queue again, add a new one.
import { QueueList } from "./lib/queue";
import { pausable } from "./lib/pause";

function Queue() {
  return <QueueList />;
}

// blank while showcase mode pauses every window
export default pausable(Queue);

// Doctor: `aleph doctor --json` by machine, the machines with problems first
// — each machine's row (its logo, its counts; a card with every check in its
// detail), then its checks, problems first: a mark, a short title, the key
// fact as a tag in its colour, the version. A check's detail: a card, why it
// matters, its fix. Pick one machine in the dropdown (`aleph --host H
// doctor`). A fix that is an aleph command runs in a terminal tab (↵); the
// others are copied.
import { DoctorList } from "./lib/doctor";
import { pausable } from "./lib/pause";

function Doctor() {
  return <DoctorList />;
}

// blank while showcase mode pauses every window
export default pausable(Doctor);

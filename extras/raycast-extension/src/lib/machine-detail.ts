// One machine's whole detail in Machines (which took Machine Stats in): its
// card with the link, the live gauges, its sessions right under them, then
// the GPUs, the cores, the live trends, what it is (with its agents), disks
// and its LLM servers (no process tables). Before the first sample (or when the stats
// can't be read), the cards Machines has without them. No Raycast imports.
import type { MachineStats } from "./cli";
import { MachineCardOpts, machineCards, machineParts } from "./machines-view";
import type { StatSample } from "./samples";
import { statsMarkdown } from "./stats-view";

export function machineDetail(
  o: MachineCardOpts & {
    stats?: MachineStats;
    statHist: StatSample[];
    /** just the card, the gauges and the sessions: drawn while you move between machines, the rest once you stop */
    light?: boolean;
  },
): string {
  const m = o.stats;
  if (!m || !m.reachable || !o.h.reachable) {
    if (!o.light) return machineCards(o);
    const p = machineParts(o);
    return [p.hero, ...p.body, ...p.sessions].join("\n\n");
  }
  // the agents are in the facts card with the stats
  const p = machineParts({ ...o, noAgents: true });
  return statsMarkdown({
    h: m,
    host: o.h,
    agents: o.a,
    hist: o.statHist,
    t: o.t,
    logo: o.logo,
    brand: o.brand,
    width: o.width,
    hero: p.hero,
    afterGauges: p.sessions,
    end: p.chips,
    procs: false,
    light: o.light,
  });
}

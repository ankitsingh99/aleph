//! Showcase mode's demo flow and the dashboard: what its autopilot tour may
//! do here (`showcase::Pilot`: show a view or sub-view, walk its list, open
//! Focus or Tile — navigation only: it never approves, sends, closes,
//! attaches or types into a session), and what presenter sync says the
//! dashboard shows (`presenting`). A build without the feature never calls
//! either.

use super::monitor_draw::Sub;
use super::{fleet, wall, Dash, Mode, View};
use crate::config::Config;
use crate::showcase::Presenting;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl Dash {
    /// What this dashboard shows, for presenter sync: its view (Monitor's
    /// sub-view by name), the machine and the session selected — Focus's
    /// own, in Focus or Tile.
    pub(crate) fn presenting(&self) -> Presenting {
        let mut p = Presenting::default();
        let session = |p: &mut Presenting, s: &crate::herdr::Session| {
            p.host = Some(s.host.clone());
            p.pane = Some(s.pane_id.clone());
            p.name = Some(s.name.clone());
        };
        match self.view {
            View::Fleet => {
                p.view = "fleet".into();
                match fleet::selected(self) {
                    Some(fleet::RowId::Sess(k)) => {
                        if let Some(s) = fleet::session(self, &k) {
                            session(&mut p, &s);
                        }
                    }
                    Some(fleet::RowId::Host(h)) => p.host = Some(h),
                    _ => {}
                }
            }
            View::Wall => {
                p.view = if self.wall.in_tile() { "tile" } else { "focus" }.into();
                if let Some(s) = self.wall.focused_session(self) {
                    session(&mut p, &s);
                }
            }
            View::Monitor => {
                p.view = match self.mon.sub {
                    Sub::Fleet => "monitor".to_string(),
                    Sub::Procs => "processes".to_string(),
                    Sub::Net => "network".to_string(),
                    s => s.name().to_lowercase(),
                };
                p.host = self.fleet.hosts.get(self.mon.sel).map(|h| h.alias.clone());
            }
            View::Inbox => p.view = "inbox".into(),
            View::Settings => p.view = "settings".into(),
        }
        p
    }
}

impl crate::showcase::Pilot for Dash {
    fn cfg(&self) -> &Config {
        &self.cfg
    }

    fn free(&self) -> bool {
        matches!(self.mode, Mode::Normal)
            && self.layout_mode.is_none()
            && !self.fv.filtering
            && !self.mon.filtering
            && !self.inbox.searching
            && !self.set.searching
            && !self.set.captures()
            && !(self.view == View::Wall && self.wall.captures())
    }

    fn show(&mut self, name: &str) -> bool {
        let sub = match name {
            "monitor" => Some(Sub::Fleet),
            "overview" => Some(Sub::Overview),
            "cpu" => Some(Sub::Cpu),
            "memory" => Some(Sub::Memory),
            "gpu" => Some(Sub::Gpu),
            "processes" => Some(Sub::Procs),
            "network" => Some(Sub::Net),
            "services" => Some(Sub::Services),
            "usage" => Some(Sub::Usage),
            _ => None,
        };
        if let Some(s) = sub {
            self.switch(View::Monitor);
            self.mon.sub = s;
            self.mon.filtering = false;
            return true;
        }
        match name {
            "fleet" => self.switch(View::Fleet),
            "inbox" => self.switch(View::Inbox),
            "settings" | "doctor" => {
                self.switch(View::Settings);
                let want = if name == "doctor" {
                    "Doctor"
                } else {
                    "General"
                };
                if let Some(i) = crate::config::SECTIONS.iter().position(|s| *s == want) {
                    super::settings::set_section(self, i);
                }
            }
            _ => return false,
        }
        true
    }

    fn walk(&mut self, first: bool) {
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        match self.view {
            // the next session (round to the first after the last)
            View::Fleet => {
                let sessions: Vec<fleet::RowId> = fleet::rows(self)
                    .into_iter()
                    .filter(|r| matches!(r, fleet::RowId::Sess(_)))
                    .collect();
                let now = fleet::selected(self);
                let i = match (first, now) {
                    (false, Some(r)) => sessions
                        .iter()
                        .position(|x| *x == r)
                        .map_or(0, |i| (i + 1) % sessions.len().max(1)),
                    _ => 0,
                };
                if let Some(r) = sessions.get(i) {
                    self.fv.sel = Some(r.clone());
                }
            }
            View::Inbox => {
                let k = key(if first { KeyCode::Home } else { KeyCode::Down });
                super::inbox::on_key(self, k);
            }
            // the next machine
            View::Monitor if !first => {
                let n = self.fleet.hosts.len().max(1);
                self.mon.sel = (self.mon.sel + 1) % n;
            }
            _ => {}
        }
    }

    fn focus(&mut self, after: Option<&str>) -> Option<String> {
        let all: Vec<(String, bool)> = self
            .fleet
            .sessions()
            .iter()
            .filter(|s| s.agent != crate::agents::SHELL)
            .map(|s| (s.key(), s.status == "working"))
            .collect();
        // working ones first, in the fleet's order; then the rest
        let mut order: Vec<&String> = all.iter().filter(|x| x.1).map(|x| &x.0).collect();
        order.extend(all.iter().filter(|x| !x.1).map(|x| &x.0));
        let working = all.iter().filter(|x| x.1).count();
        let pool = if working > 0 {
            &order[..working]
        } else {
            &order[..]
        };
        let i = after
            .and_then(|a| pool.iter().position(|k| *k == a))
            .map_or(0, |i| (i + 1) % pool.len().max(1));
        let key = (*pool.get(i)?).clone();
        let from = if self.view == View::Wall {
            View::Fleet
        } else {
            self.view
        };
        wall::focus_on(self, &key, from);
        // the tour only looks: Aleph's keys, never the agent's
        self.wall.typing = false;
        Some(key)
    }

    fn tile(&mut self, after: Option<&str>) -> Option<String> {
        let hosts: Vec<(String, Vec<String>)> = self
            .fleet
            .hosts
            .iter()
            .map(|h| {
                let keys = h
                    .sessions
                    .iter()
                    .filter(|s| s.agent != crate::agents::SHELL)
                    .map(|s| s.key())
                    .collect::<Vec<_>>();
                (h.alias.clone(), keys)
            })
            .filter(|(_, k)| k.len() >= 2)
            .collect();
        let i = after
            .and_then(|a| hosts.iter().position(|(h, _)| h == a))
            .map_or(0, |i| (i + 1) % hosts.len().max(1));
        let (host, keys) = hosts.get(i)?.clone();
        wall::tile(self, keys, View::Fleet);
        Some(host)
    }
}

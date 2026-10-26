//! Attach in showcase mode: herdr's own screen (and an editor on the
//! settings file) draws what it likes, where the lens can't reach. So `a`
//! and enter open Aleph's Focus, which is masked; `A` and `O` attach for
//! real after a question; in the demo world there's nothing real to attach
//! to at all.

use super::state::{self, Mode};
use super::{Gate, Outside};

pub fn gate(what: Outside) -> Gate {
    let demo = state::mode() == Mode::Demo;
    match what {
        Outside::Attach => Gate::Focus(
            "showcase: a stays in Focus, which is masked — A attaches in herdr's own screen",
        ),
        Outside::Herdr if demo => Gate::Refuse(
            "the demo world has nothing real to attach to — enter opens a session in Focus",
        ),
        Outside::Herdr => Gate::Ask {
            title: "Attach in herdr?",
            text: "herdr's own screen isn't masked — attach anyway?",
        },
        Outside::Editor if demo => {
            Gate::Refuse("the demo world keeps the settings file shut — Settings shows them, masked")
        }
        Outside::Editor => Gate::Ask {
            title: "Open the settings file?",
            text: "your editor shows the file as it is: names, addresses and tokens aren't masked — open it anyway?",
        },
    }
}

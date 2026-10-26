//! The worlds stand-ins come from, each with its names sorted by what they
//! are: AIs and computers, people, ships and stations, places, and things
//! (tech, artefacts, lines everyone knows). Settings › Showcase picks which
//! universes are in play (`showcase.universes`), whether they mix or one
//! rules each launch (`showcase.theme`), and which kinds of name each field
//! gets (`showcase.machines` / `.users` / `.things`, plus your own words).
//!
//! Names are lowercase ASCII, words joined by `-` (digits allowed); a
//! stand-in takes the real word's separator and case.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cat {
    Ais,
    People,
    Ships,
    Places,
    Things,
}

impl Cat {
    pub const ALL: [Cat; 5] = [Cat::Ais, Cat::People, Cat::Ships, Cat::Places, Cat::Things];

    pub fn id(self) -> &'static str {
        match self {
            Cat::Ais => "ais",
            Cat::People => "people",
            Cat::Ships => "ships",
            Cat::Places => "places",
            Cat::Things => "things",
        }
    }
}

pub struct Universe {
    pub id: &'static str,
    pub title: &'static str,
    /// classic, space, games or films (how Settings groups them)
    pub group: &'static str,
    pub ais: &'static [&'static str],
    pub people: &'static [&'static str],
    pub ships: &'static [&'static str],
    pub places: &'static [&'static str],
    pub things: &'static [&'static str],
}

impl Universe {
    pub fn names(&self, c: Cat) -> &'static [&'static str] {
        match c {
            Cat::Ais => self.ais,
            Cat::People => self.people,
            Cat::Ships => self.ships,
            Cat::Places => self.places,
            Cat::Things => self.things,
        }
    }
}

pub fn find(id: &str) -> Option<&'static Universe> {
    ALL.iter().find(|u| u.id.eq_ignore_ascii_case(id))
}

#[rustfmt::skip]
pub const ALL: &[Universe] = &[
    Universe {
        id: "hhgttg", title: "The Hitchhiker's Guide to the Galaxy", group: "classic",
        ais: &["deep-thought", "eddie", "marvin", "earth", "the-guide", "hactar", "colin", "nutrimatic"],
        people: &["arthur-dent", "ford-prefect", "zaphod", "trillian", "slartibartfast", "fenchurch",
            "random", "jeltz", "agrajag", "wowbagger", "zarniwoop", "hotblack", "roosta", "majikthise",
            "vroomfondel", "lunkwill", "fook", "loonquawl", "phouchg", "eccentrica"],
        ships: &["heart-of-gold", "bistromath", "b-ark", "starship-titanic", "vogon-fleet", "tanngrisnir"],
        places: &["magrathea", "milliways", "betelgeuse", "vogsphere", "krikkit", "frogstar", "damogran",
            "santraginus", "ursa-minor", "lamuella", "viltvodle", "kakrafoon", "brontitall",
            "squornshellous", "jaglan-beta", "han-wavel", "golgafrincham", "traal", "earth-mk-ii"],
        things: &["towel", "babel-fish", "gargle-blaster", "improbability-drive", "perspective-vortex",
            "bugblatter", "answer-42", "dont-panic", "mostly-harmless", "so-long", "sub-etha",
            "joo-janta-200", "point-of-view-gun", "bowl-of-petunias", "the-ultimate-question"],
    },
    Universe {
        id: "trek", title: "Star Trek", group: "classic",
        ais: &["data", "lore", "the-doctor", "zora", "nomad", "vger", "m-5", "landru", "moriarty",
            "peanut-hamper", "badgey", "control"],
        people: &["picard", "kirk", "spock", "janeway", "sisko", "uhura", "worf", "riker", "troi",
            "crusher", "laforge", "scotty", "sulu", "chekov", "mccoy", "seven-of-nine", "quark", "odo",
            "kira", "dax", "garak", "archer", "burnham", "pike", "saru", "tuvok", "chakotay", "guinan",
            "neelix", "boimler", "mariner", "tendi"],
        ships: &["enterprise", "voyager", "defiant", "excelsior", "stargazer", "discovery", "cerritos",
            "reliant", "intrepid", "titan", "protostar", "ds9", "deep-space-9", "yorktown", "spacedock",
            "k-7", "farpoint", "ncc-1701"],
        places: &["vulcan", "qonos", "romulus", "bajor", "cardassia", "risa", "ferenginar", "betazed",
            "andoria", "talos", "rigel", "khitomer", "ceti-alpha", "wolf-359", "nimbus-iii",
            "neutral-zone", "delta-quadrant", "gamma-quadrant", "genesis"],
        things: &["warp-core", "tricorder", "phaser", "transporter", "replicator", "holodeck", "dilithium",
            "batleth", "tribble", "isolinear", "make-it-so", "engage", "prime-directive",
            "kobayashi-maru", "red-alert", "earl-grey-hot", "live-long", "subspace"],
    },
    Universe {
        id: "starwars", title: "Star Wars", group: "classic",
        ais: &["r2-d2", "c-3po", "bb-8", "k-2so", "ig-88", "l3-37", "chopper", "huyang", "hk-47",
            "bd-1", "ig-11", "d-o", "4-lom"],
        people: &["luke", "leia", "han", "chewie", "vader", "obi-wan", "yoda", "rey", "finn", "poe",
            "kylo", "ahsoka", "din-djarin", "grogu", "lando", "padme", "anakin", "tarkin", "cassian",
            "jyn", "hera", "ezra", "sabine", "boba", "qui-gon", "mace", "thrawn", "revan"],
        ships: &["millennium-falcon", "x-wing", "star-destroyer", "tantive-iv", "razor-crest", "slave-i",
            "ghost", "executor", "home-one", "death-star", "starkiller-base", "ebon-hawk", "outrider",
            "a-wing", "tie-fighter"],
        places: &["tatooine", "hoth", "endor", "naboo", "dagobah", "coruscant", "bespin", "kashyyyk",
            "mustafar", "jakku", "scarif", "jedha", "yavin", "lothal", "mandalore", "corellia", "kessel",
            "geonosis", "kamino", "utapau", "exegol", "alderaan", "ahch-to", "crait", "mos-eisley",
            "cloud-city"],
        things: &["lightsaber", "kyber", "hyperdrive", "holocron", "blaster", "beskar", "the-force",
            "kessel-run", "this-is-the-way", "the-odds", "may-the-force", "bacta", "carbonite",
            "thermal-detonator"],
    },
    Universe {
        id: "odyssey", title: "2001: A Space Odyssey", group: "classic",
        ais: &["hal-9000", "sal-9000", "hal"],
        people: &["dave-bowman", "frank-poole", "heywood-floyd", "chandra", "curnow", "kirbuk",
            "moonwatcher"],
        ships: &["discovery-one", "leonov", "space-station-v", "aries-1b", "orion-iii", "galaxy",
            "universe"],
        places: &["clavius", "tycho", "jupiter", "europa", "io", "ganymede", "lucifer", "iapetus"],
        things: &["monolith", "tma-1", "pod-bay-doors", "daisy", "star-child", "star-gate",
            "dawn-of-man", "full-of-stars", "eva-pod"],
    },
    Universe {
        id: "borges", title: "Borges' Aleph (the point that holds every other)", group: "classic",
        ais: &["the-aleph", "the-zahir", "funes"],
        people: &["borges", "beatriz-viterbo", "carlos-argentino", "funes", "pierre-menard",
            "yu-tsun", "herbert-quain"],
        ships: &["the-cellar", "calle-garay", "the-library", "the-labyrinth"],
        places: &["tlon", "uqbar", "orbis-tertius", "babel", "the-library-of-babel", "the-city-of-immortals"],
        things: &["aleph", "zahir", "forking-paths", "babel-lottery", "the-immortal", "hexagon",
            "every-point", "all-points", "aleph-null", "the-book-of-sand"],
    },
    Universe {
        id: "cosmos", title: "The cosmos", group: "space",
        ais: &[],
        people: &[],
        ships: &["hubble", "webb", "chandra", "kepler", "gaia", "spitzer", "planck", "cassini", "juno",
            "rosetta", "voyager-1", "voyager-2", "new-horizons", "pioneer-10", "curiosity",
            "perseverance", "ingenuity", "galileo", "magellan", "dawn", "philae", "huygens", "lisa"],
        places: &["sirius", "vega", "rigel", "polaris", "altair", "deneb", "antares", "arcturus",
            "aldebaran", "proxima", "trappist-1", "kepler-22b", "andromeda", "triangulum", "sombrero",
            "orion-nebula", "crab-nebula", "pillars-of-creation", "eagle-nebula", "horsehead", "helix",
            "cats-eye", "carina", "magellanic", "sagittarius-a", "m87", "cygnus-x1", "3c-273",
            "vela-pulsar", "titan", "enceladus", "callisto", "triton", "charon", "ceres", "vesta",
            "pluto", "oort-cloud", "kuiper-belt", "laniakea"],
        things: &["quasar", "pulsar", "magnetar", "blazar", "nebula", "supernova", "hypernova",
            "kilonova", "neutron-star", "white-dwarf", "red-giant", "brown-dwarf", "black-hole",
            "event-horizon", "accretion-disc", "photon-ring", "wormhole", "dark-matter", "dark-energy",
            "redshift", "parallax", "parsec", "light-year", "cosmic-dawn", "big-bang", "inflation",
            "gravitational-wave", "solar-wind", "aurora", "perihelion", "lagrange-point", "roche-limit",
            "spaghettification", "singularity", "hawking-radiation", "the-aleph"],
    },
    Universe {
        id: "spaceflight", title: "Spaceflight & astronomy", group: "space",
        ais: &["agc", "eniac"],
        people: &["gagarin", "tereshkova", "armstrong", "aldrin", "collins", "glenn", "shepard",
            "sally-ride", "jemison", "hadfield", "lovell", "leonov", "korolev", "katherine-johnson",
            "dorothy-vaughan", "mary-jackson", "margaret-hamilton", "hubble", "sagan", "hawking",
            "galileo", "kepler", "copernicus", "tycho-brahe", "newton", "einstein", "chandrasekhar",
            "vera-rubin", "leavitt", "cecilia-payne", "jocelyn-bell", "hypatia", "herschel", "laika",
            "yang-liwei", "rakesh-sharma", "kalpana-chawla", "sunita-williams", "peake"],
        ships: &["vostok-1", "apollo-11", "eagle", "columbia", "challenger", "endeavour", "atlantis",
            "soyuz", "shenzhou", "crew-dragon", "starship", "gaganyaan", "chandrayaan", "mangalyaan",
            "artemis", "orion", "skylab", "mir", "iss", "tiangong", "sputnik"],
        places: &["baikonur", "cape-canaveral", "kennedy", "houston", "sriharikota", "tranquility-base",
            "star-city", "jodrell-bank", "arecibo", "mauna-kea", "atacama", "palomar", "greenwich"],
        things: &["one-small-step", "we-have-a-problem", "poyekhali", "pale-blue-dot", "earthrise",
            "eagle-has-landed", "godspeed", "liftoff", "t-minus-10", "max-q", "splashdown",
            "not-an-option"],
    },
    Universe {
        id: "remedy", title: "Remedy & noir (Alan Wake, Max Payne, Control, Quantum Break)", group: "games",
        ais: &["the-board", "polaris", "hedron", "dark-presence", "the-hiss"],
        people: &["alan-wake", "alice-wake", "barry-wheeler", "saga-anderson", "thomas-zane",
            "max-payne", "mona-sax", "vlad", "vinnie", "jesse-faden", "dylan-faden", "emily-pope",
            "ahti", "darling", "trench", "jack-joyce", "paul-serene", "beth-wilder", "mr-door"],
        ships: &["oldest-house", "aesir-tower", "monarch-tower", "cauldron-lodge", "panopticon"],
        places: &["bright-falls", "cauldron-lake", "dark-place", "watery", "ashtray-maze",
            "research-sector", "ragnarock", "night-springs", "riverside"],
        things: &["thermos", "flashlight", "the-clicker", "service-weapon", "altered-item",
            "object-of-power", "valkyr", "bullet-time", "chronon", "its-an-ocean", "old-gods",
            "take-control", "the-typewriter"],
    },
    Universe {
        id: "valve", title: "Valve & immersive sims (Half-Life, Portal, System Shock, Deus Ex, BioShock, Prey)", group: "games",
        ais: &["glados", "wheatley", "shodan", "xerxes", "daedalus", "icarus", "helios", "morpheus",
            "january", "december", "caroline"],
        people: &["gordon-freeman", "alyx-vance", "eli-vance", "barney-calhoun", "g-man", "kleiner",
            "breen", "chell", "cave-johnson", "rattmann", "jc-denton", "paul-denton", "adam-jensen",
            "andrew-ryan", "atlas", "elizabeth", "booker", "sander-cohen", "morgan-yu", "alex-yu"],
        ships: &["black-mesa", "aperture", "the-citadel", "borealis", "von-braun", "rickenbacker",
            "citadel-station", "talos-1", "rapture", "columbia"],
        places: &["city-17", "ravenholm", "nova-prospekt", "white-forest", "xen", "lambda-core",
            "enrichment-center", "liberty-island", "panchaea", "hengsha"],
        things: &["crowbar", "gravity-gun", "portal-gun", "companion-cube", "cake-is-a-lie",
            "headcrab", "lambda", "resonance-cascade", "plasmid", "would-you-kindly", "neuromod",
            "gloo-cannon", "rise-and-shine", "augmentation"],
    },
    Universe {
        id: "spacegames", title: "Space games (Mass Effect, Halo, Dead Space, Outer Wilds, Homeworld, StarCraft, FTL, Kerbal)", group: "games",
        ais: &["edi", "cortana", "guilty-spark", "legion", "vigil", "ghost", "atlas", "karan-sjet"],
        people: &["shepard", "garrus", "tali", "liara", "wrex", "mordin", "joker", "master-chief",
            "johnson", "arbiter", "isaac-clarke", "nicole", "kerrigan", "raynor", "zeratul", "artanis",
            "jebediah", "bill-kerman", "bob-kerman", "valentina", "feldspar", "chert", "esker",
            "riebeck", "gabbro", "hornfels"],
        ships: &["normandy", "normandy-sr2", "the-citadel", "pillar-of-autumn", "infinity",
            "forward-unto-dawn", "ishimura", "kestrel", "hyperion", "mothership", "kerbal-x",
            "coriolis", "cobra-mk3", "space-anomaly"],
        places: &["tuchanka", "omega", "illium", "rannoch", "thessia", "palaven", "reach",
            "installation-04", "sanghelios", "aiur", "char", "korhal", "mar-sara", "kharak", "hiigara",
            "timber-hearth", "brittle-hollow", "giants-deep", "dark-bramble", "ember-twin", "ash-twin",
            "quantum-moon", "kerbin", "mun", "minmus", "duna", "jool", "laythe", "aegis-vii",
            "tau-volantis"],
        things: &["mass-relay", "element-zero", "omni-tool", "spartan", "energy-sword", "warthog",
            "plasma-cutter", "the-marker", "nomai", "eye-of-the-universe", "zerg-rush", "fuel-scoop",
            "moar-boosters", "snacks", "ftl-drive", "the-flagship", "marshmallow"],
    },
    Universe {
        id: "retro", title: "Retro & cult classics (Metroid, Doom, Quake, Myst, Metal Gear, Chrono Trigger, Thief)", group: "games",
        ais: &["mother-brain", "adam", "robo", "gw", "the-patriots", "mu-zak"],
        people: &["samus", "ridley", "kraid", "doomguy", "ranger", "atrus", "catherine", "sirrus",
            "achenar", "snake", "otacon", "meryl", "ocelot", "big-boss", "the-boss", "raiden", "crono",
            "marle", "lucca", "frog", "magus", "garrett", "fox-mccloud", "falco", "slippy", "peppy",
            "guybrush", "elaine", "lechuck", "manny-calavera", "cloud", "aerith", "tifa"],
        ships: &["gunship", "great-fox", "arwing", "epoch", "shadow-moses", "outer-heaven",
            "highwind", "sea-monkey", "phobos-base", "deimos-base", "uac", "vault-101"],
        places: &["zebes", "tallon-iv", "sr388", "phobos", "deimos", "myst-island", "riven", "dni",
            "zanzibar-land", "guardia", "zeal", "end-of-time", "corneria", "venom", "melee-island",
            "monkey-island", "midgar", "the-city", "sigil"],
        things: &["varia-suit", "morph-ball", "screw-attack", "bfg9000", "quad-damage", "linking-book",
            "cardboard-box", "codec", "nanomachines", "master-sword", "water-arrow", "materia",
            "buster-sword", "grog", "kept-you-waiting", "dangerous-to-go-alone", "rip-and-tear"],
    },
    Universe {
        id: "scifi", title: "Classic sci-fi films (Blade Runner, Alien, Terminator, The Matrix, Back to the Future)", group: "films",
        ais: &["mother", "ash", "bishop", "skynet", "the-architect", "the-oracle", "agent-smith",
            "robby", "maria", "joshua", "wopr", "mcp", "samantha", "ava", "auto", "wall-e", "eve",
            "johnny-5", "ed-209", "t-800", "t-1000", "colossus"],
        people: &["deckard", "roy-batty", "rachael", "gaff", "ripley", "dallas", "newt", "hicks",
            "sarah-connor", "john-connor", "kyle-reese", "neo", "trinity", "morpheus", "marty-mcfly",
            "doc-brown", "biff", "leeloo", "korben-dallas", "zorg", "flynn", "murphy", "quaid",
            "macready", "elliott"],
        ships: &["nostromo", "sulaco", "nebuchadnezzar", "delorean", "spinner", "fhloston", "axiom",
            "recognizer", "solar-sailer", "the-mothership"],
        places: &["lv-426", "zion", "the-matrix", "hill-valley", "tyrell-corp", "off-world",
            "tannhauser-gate", "the-grid", "cyberdyne", "delta-city", "devils-tower"],
        things: &["voight-kampff", "replicant", "tears-in-rain", "flux-capacitor", "hoverboard",
            "gigawatts", "red-pill", "blue-pill", "no-spoon", "ill-be-back", "xenomorph",
            "facehugger", "pulse-rifle", "power-loader", "light-cycle", "identity-disc", "multipass",
            "fifth-element", "great-scott", "game-over-man"],
    },
    Universe {
        id: "spacefilms", title: "Modern space films (Interstellar, Dune, Arrival, Contact, The Martian, Moon)", group: "films",
        ais: &["tars", "case", "kipp", "gerty", "the-tet", "arthur", "icarus", "david-8"],
        people: &["cooper", "murph", "brand", "mann", "paul-atreides", "chani", "stilgar", "leto",
            "jessica", "duncan-idaho", "gurney", "louise-banks", "ian-donnelly", "ellie-arroway",
            "mark-watney", "melissa-lewis", "sam-bell", "ryan-stone", "kowalski", "roy-mcbride",
            "capa", "lena", "jake-sully", "neytiri"],
        ships: &["endurance", "ranger", "lazarus", "hermes", "ares-iii", "icarus-ii", "explorer",
            "avalon", "event-horizon", "lewis-and-clark", "prometheus", "covenant", "sarang",
            "the-shell"],
        places: &["gargantua", "millers-planet", "manns-planet", "edmunds-planet", "cooper-station",
            "arrakis", "caladan", "giedi-prime", "sietch-tabr", "the-shimmer", "pandora", "acidalia",
            "schiaparelli", "the-tesseract"],
        things: &["tesseract", "spice", "melange", "thumper", "sandworm", "crysknife", "the-voice",
            "gom-jabbar", "water-of-life", "heptapod", "logogram", "the-machine", "time-dilation",
            "go-gentle", "potatoes", "spice-must-flow", "mind-killer", "unobtanium"],
    },
];

/// Every name of the kinds `cats` from the universes `ids` (all when empty).
pub fn names(ids: &[&str], cats: &[Cat]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for u in ALL.iter().filter(|u| ids.is_empty() || ids.contains(&u.id)) {
        for c in cats {
            for n in u.names(*c) {
                if !out.contains(n) {
                    out.push(n);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_plain_lowercase_words_joined_by_dashes() {
        for u in ALL {
            for c in Cat::ALL {
                for n in u.names(c) {
                    assert!(
                        !n.is_empty()
                            && n.bytes()
                                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                            && !n.starts_with('-')
                            && !n.ends_with('-')
                            && !n.contains("--"),
                        "{}: {n}",
                        u.id
                    );
                }
            }
        }
    }

    #[test]
    fn every_universe_has_ships_or_places_and_people_or_ais() {
        for u in ALL {
            assert!(!u.ships.is_empty() || !u.places.is_empty(), "{}", u.id);
            assert!(!u.things.is_empty(), "{}", u.id);
            assert!(u.id.len() <= 12 && find(u.id).is_some());
        }
        assert!(names(&["valve"], &[Cat::Ais]).contains(&"glados"));
        assert!(!names(&["valve"], &[Cat::Ais]).contains(&"normandy"));
    }
}

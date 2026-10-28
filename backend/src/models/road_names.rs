//! Road names (#483): what people call the roads, so a lot has an address.
//!
//! Every road a lot fronts gets a name, drawn from the world's seed once the
//! world is laid, in the manner of its zone:
//! - **The great roads** (the arterials) are named for what came in along them
//!   from the country: the Timber Road, the Drovers' Road. A road keeps its
//!   name across the canal, and its trail in the wilds takes it on ("Timber
//!   Trail"). The rings are the Guild Ring and the Trade Ring, and the south
//!   road's last stretch to the tower is Capitol Avenue.
//! - **The urban grid** is a city of trades: the quay is Canal Quay, the ring
//!   streets are "Mercer Street", and the spokes are gates in the old northern
//!   sense of a street: "Tannergate", "Ropergate".
//! - **The suburb** is post-war estates. Each neighbourhood gets a theme, as a
//!   builder's estate did (the tree estate, the bird estate, the star estate),
//!   and each street a word from it with the suffix its shape asks for: a
//!   collector is a Drive or a Crescent when it curves, an Avenue or Gardens
//!   when it doesn't, a cul-de-sac a Close or a Court, a link a Walk.
//! - **The rural ring** follows its sectors' characters. A concession sector's
//!   straight roads were surveyed, and are named for the families who took up
//!   the first lots: "Hallett Line", with "Jessop Sideroad" between. A winding
//!   sector's are old lanes: "Sheepwash Lane", "Cuckoo Drove".
//! - **The wilds**' passes are Edges and their climbs Rakes, after the high
//!   country's birds and weather. The spurs to the lakes and fords are tracks
//!   nobody names.
//!
//! Names are unique across the world, but for a great road's stretches,
//! which share one. Each family draws from a stream of its own
//! (`SplitMix64::stream`), so the same seed always names the world alike.
//!
//! Then every lot has an address ([`addresses`]): its number along its
//! street, odd on the left and even on the right, and on a street that runs
//! round the tower its nearest crossing too: "3 Sheepwash Lane", "112 Mercer
//! Street by Tannergate". The Gazette and the Deeds Office say it.

use std::collections::{HashMap, HashSet};

use capitol_protocol::terrain::{RoadSegment, TerrainPlot};

use super::geom::SplitMix64;
use super::rural::sectors::{Character, Sector};

type P = (f64, f64);

/// The names' streams, apart from every other stream off the world's seed.
const SALT: u64 = 0x0483_4e41_4d45;

/// The road in's word: the south road every newcomer walks up to the hall
const PILGRIMS: &str = "Pilgrims'";

/// What the great roads carried in, one each, by the seed.
const GREAT_ROADS: [&str; 10] =
    ["Timber", "Drovers'", "Harvest", "Wool", "Quarry", "Flax", "Kiln", "Iron", "Carters'", "Pilgrims'"];

/// The trades of the urban grid. The ring streets take the first few as
/// Streets, and the spokes the rest as gates.
const TRADES: [&str; 56] = [
    "Tanner", "Cooper", "Fisher", "Smith", "Glover", "Fletcher", "Bower", "Roper", "Weaver", "Fuller", "Dyer",
    "Mercer", "Draper", "Potter", "Turner", "Joiner", "Sawyer", "Mason", "Thatcher", "Tiler", "Slater", "Carter",
    "Wheeler", "Wright", "Saddler", "Spurrier", "Cutler", "Pewterer", "Brazier", "Lorimer", "Collier", "Miller",
    "Baxter", "Brewer", "Vintner", "Salter", "Fowler", "Skinner", "Shearer", "Comber", "Spinner", "Webster",
    "Tailor", "Hatter", "Lister", "Barker", "Glazier", "Carver", "Chandler", "Corder", "Hosier", "Needler",
    "Cordwainer", "Furrier", "Ostler", "Tinker",
];

/// The suburb's estates, one theme to a neighbourhood.
const ESTATES: [&[&str]; 14] = [
    // The tree estate
    &[
        "Larch", "Rowan", "Hazel", "Alder", "Birch", "Aspen", "Hornbeam", "Linden", "Sycamore", "Chestnut", "Walnut",
        "Elder", "Juniper", "Laurel", "Maple", "Poplar", "Willow", "Yew", "Cedar", "Beech", "Ash", "Oak", "Elm",
        "Spruce", "Whitebeam", "Cherry", "Mulberry",
    ],
    // The bird estate
    &[
        "Wren", "Linnet", "Thrush", "Blackbird", "Robin", "Goldfinch", "Goldcrest", "Siskin", "Warbler", "Nightingale",
        "Skylark", "Chiffchaff", "Redstart", "Bunting", "Dunnock", "Starling", "Swallow", "Martin", "Swift",
        "Fieldfare", "Redwing", "Waxwing", "Nuthatch", "Whitethroat", "Wagtail", "Greenfinch", "Treecreeper",
    ],
    // The wildflower estate
    &[
        "Primrose", "Cowslip", "Bluebell", "Foxglove", "Campion", "Celandine", "Meadowsweet", "Harebell", "Speedwell",
        "Cornflower", "Poppy", "Clover", "Yarrow", "Sorrel", "Teasel", "Valerian", "Vetch", "Toadflax", "Buttercup",
        "Daisy", "Violet", "Woodruff", "Betony", "Scabious", "Tansy", "Knapweed", "Oxlip",
    ],
    // The orchard estate
    &[
        "Bramley", "Russet", "Pippin", "Codlin", "Pearmain", "Costard", "Nonpareil", "Quince", "Damson", "Greengage",
        "Medlar", "Bullace", "Mirabelle", "Blenheim", "Ribston", "Catshead", "Pomona", "Warden", "Jargonelle",
        "Bergamot", "Perry", "Cider", "Orchard", "Blossom", "Espalier", "Sweeting", "Leathercoat",
    ],
    // The herb garden
    &[
        "Rosemary", "Thyme", "Sage", "Fennel", "Lovage", "Chervil", "Tarragon", "Marjoram", "Borage", "Hyssop",
        "Angelica", "Comfrey", "Feverfew", "Lavender", "Mint", "Basil", "Savory", "Rue", "Dill", "Caraway",
        "Coriander", "Chamomile", "Balm", "Sweet Cicely", "Burnet", "Parsley", "Woad",
    ],
    // The old calendar
    &[
        "Candlemas", "Ladyday", "Lammas", "Michaelmas", "Martinmas", "Whitsun", "Midsummer", "Hallowtide",
        "Hocktide", "Shrovetide", "Rogation", "Advent", "Eastertide", "Yuletide", "Wassail", "Maytime", "Haysel",
        "Lent", "Ember", "Plough Monday", "Lantern", "Harvest Home", "St Swithin", "Twelfth Night", "Mayday",
    ],
    // The stone estate
    &[
        "Flint", "Chalk", "Slate", "Granite", "Marl", "Sarsen", "Basalt", "Jasper", "Agate", "Garnet", "Quartz",
        "Ironstone", "Greensand", "Clunch", "Cobble", "Shingle", "Gault", "Tufa", "Oolite", "Gritstone", "Porphyry",
        "Onyx", "Mica", "Feldspar", "Lodestone", "Carnelian", "Beryl",
    ],
    // The waterside
    &[
        "Heron", "Kingfisher", "Moorhen", "Mallard", "Teal", "Plover", "Sandpiper", "Dipper", "Otter", "Reed", "Rush",
        "Sedge", "Osier", "Bulrush", "Watercress", "Lily", "Weir", "Brook", "Ford", "Pool", "Fen", "Mere",
        "Wellspring", "Grebe", "Coot", "Damselfly", "Mayfly",
    ],
    // The butterfly estate
    &[
        "Admiral", "Brimstone", "Peacock", "Comma", "Fritillary", "Ringlet", "Gatekeeper", "Skipper", "Hairstreak",
        "Orange Tip", "Holly Blue", "Painted Lady", "Marbled White", "Clouded Yellow", "Emperor", "Tortoiseshell",
        "Grayling", "Argus", "Copper", "Swallowtail", "Hawkmoth", "Tiger Moth", "Ermine", "Lappet", "Footman",
        "Speckled Wood", "Wall Brown",
    ],
    // The star estate, as the fifties named them
    &[
        "Orion", "Pleiades", "Vega", "Sirius", "Capella", "Arcturus", "Polaris", "Lyra", "Cygnus", "Altair", "Deneb",
        "Rigel", "Castor", "Pollux", "Aldebaran", "Spica", "Antares", "Andromeda", "Cassiopeia", "Perseus", "Draco",
        "Aquila", "Corona", "Bellatrix", "Regulus", "Mira", "Procyon",
    ],
    // The hedgerow
    &[
        "Blackthorn", "Hawthorn", "Bramble", "Dog Rose", "Sloe", "Elderberry", "Rosehip", "Honeysuckle", "Ivy", "Hop",
        "Briar", "Spindle", "Wayfaring", "Guelder", "Bryony", "Traveller's Joy", "Holly", "Privet", "Box", "Cobnut",
        "Filbert", "Whitethorn", "Sweetbriar", "Eglantine", "Crab Apple", "Field Maple", "Dogwood",
    ],
    // The garden flowers, as the new gardens grew them
    &[
        "Lupin", "Delphinium", "Hollyhock", "Sweet Pea", "Marigold", "Aster", "Dahlia", "Peony", "Phlox", "Larkspur",
        "Snapdragon", "Wallflower", "Carnation", "Pansy", "Lilac", "Laburnum", "Magnolia", "Wisteria", "Clematis",
        "Jasmine", "Camellia", "Azalea", "Fuchsia", "Gladiolus", "Sweet William", "Stock", "Forget-me-not",
    ],
    // The virtues, as the building societies and chapels named them
    &[
        "Concord", "Providence", "Prosperity", "Temperance", "Harmony", "Felicity", "Charity", "Patience",
        "Constance", "Fortitude", "Liberty", "Amity", "Endeavour", "Industry", "Thrift", "Diligence", "Hope", "Faith",
        "Mercy", "Grace", "Plenty", "Bounty", "Welcome", "Goodwill", "Fellowship", "Unity", "Content",
    ],
    // The woodland
    &[
        "Badger", "Fox", "Hare", "Vole", "Dormouse", "Hedgehog", "Squirrel", "Fallow", "Roe", "Stoat", "Weasel",
        "Mole", "Shrew", "Polecat", "Marten", "Wildcat", "Coney", "Doe", "Fawn", "Hart", "Hind", "Brock", "Reynard",
        "Owlet", "Bracken", "Acorn", "Beechmast",
    ],
];

/// The families who took up the concession sectors' first lots.
const SETTLERS: [&str; 52] = [
    "Hallett", "Prentice", "Ormerod", "Tullis", "Cawthorne", "Aske", "Belding", "Carrow", "Dunstan", "Elwood",
    "Fenwick", "Garside", "Haddow", "Ibbotson", "Jessop", "Kettle", "Lowther", "Marriott", "Nettleship", "Oddie",
    "Pursglove", "Quarrie", "Rudd", "Swithin", "Thwaite", "Upcott", "Varley", "Whitlock", "Yeoman", "Ashdown",
    "Coultas", "Dimmock", "Gedge", "Hebden", "Inchbald", "Judd", "Kitto", "Ledger", "Mawby", "Nunn", "Orrell",
    "Pybus", "Rigg", "Scarth", "Tebbutt", "Utley", "Venn", "Wardle", "Bellamy", "Crabtree", "Fairbairn", "Mossop",
];

/// The winding sectors' lanes.
const LANES: [&str; 52] = [
    "Bramble", "Hollow", "Sheepwash", "Cuckoo", "Mill", "Watery", "Sloe", "Honeysuckle", "Foxhole", "Dovecote",
    "Withy", "Pightle", "Coldharbour", "Nightingale", "Hayfield", "Clover", "Barley", "Linnet", "Spinney", "Stile",
    "Rookery", "Ploughman's", "Miller's", "Gleaners'", "Thresher's", "Tinker's", "Shepherd's", "Cowslip",
    "Lapwing", "Owlet", "Puddle", "Sheepcote", "Swallowfield", "Mallow", "Peasecod", "Oxlip", "Bell", "Chapel",
    "Haycock", "Hopyard", "Maltings", "Moss", "Pond", "Green", "Lark", "Primrose", "Gypsy", "Sandy", "Mud",
    "Tithe Barn", "Wheelwright's", "Starveacre",
];

/// The high country's birds and weather, for the passes and the climbs.
const UPLAND: [&str; 26] = [
    "Raven", "Curlew", "Merlin", "Kestrel", "Buzzard", "Ptarmigan", "Grouse", "Heather", "Bilberry", "Cloudberry",
    "Cairn", "Windy", "Mist", "Snow", "Thunder", "Cold", "High", "Blue", "Peregrine", "Golden", "Ring Ouzel",
    "Wheatear", "Cotton Grass", "Hailstone", "Dunlin", "Lantern",
];

/// What a name is given when its word is taken in that form already: the
/// old roads' way of telling two of a name apart.
const ELDER: [&str; 6] = ["Old", "New", "Upper", "Lower", "Little", "Great"];

/// Name every road in `roads` that people would name, from the world's
/// `seed`. `rural` is the rural ring's sectors, each with `mains` main roads.
pub fn name_roads(roads: &mut [RoadSegment], seed: u64, rural: &[Sector], mains: usize) {
    let mut names = Names::default();

    // The fixed ones
    for road in roads.iter_mut() {
        let fixed = match road.id.as_str() {
            "guild_ring" => "Guild Ring",
            "trade_ring" => "Trade Ring",
            "arterial_capitol" => "Capitol Avenue",
            "bazaar_lane" => "Bazaar Lane",
            "bazaar_alley" => "Pedlars' Alley",
            "urb_ring_0" => "Canal Quay",
            _ => continue,
        };
        road.name = Some(names.take(fixed.to_owned()));
    }

    // The great roads, by their bridges, and their stretches in to the
    // guild ring with them. The road in, the one Capitol Avenue goes on
    // from, is always the Pilgrims' Road: every newcomer walks it.
    let bridge = |id: &str| id.strip_prefix("arterial_inner_").or_else(|| id.strip_prefix("arterial_"))?.parse::<usize>().ok();
    let avenue = roads.iter().find(|r| r.id == "arterial_capitol").and_then(|r| r.points.first().copied());
    let road_in = avenue.and_then(|at| {
        roads
            .iter()
            .filter(|r| r.id.starts_with("arterial_") && !r.id.starts_with("arterial_inner_"))
            .filter_map(|r| Some((bridge(&r.id)?, dist2(*r.points.first()?, at))))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });
    let mut rng = SplitMix64::stream(seed, &[SALT, 0]);
    let great: Vec<&str> = shuffled(&GREAT_ROADS, &mut rng).into_iter().filter(|w| *w != PILGRIMS).collect();
    for road in roads.iter_mut() {
        let Some(i) = bridge(&road.id) else { continue };
        // The others take the rest in their bridges' order, so the road in
        // takes no other road's word
        let word = match road_in {
            Some(r) if r == i => PILGRIMS,
            Some(r) if i > r => great[(i - 1) % great.len()],
            _ => great[i % great.len()],
        };
        let name = format!("{word} Road");
        names.used.insert(name.clone());
        road.name = Some(name);
    }

    // The wild trails take on the great road they leave
    let ends: Vec<(&str, P)> = roads
        .iter()
        .filter(|r| r.id.starts_with("arterial_") && !r.id.starts_with("arterial_inner_") && r.id != "arterial_capitol")
        .filter_map(|r| Some((r.name.as_deref()?.strip_suffix(" Road")?, *r.points.last()?)))
        .collect();
    let trail_names: Vec<Option<String>> = roads
        .iter()
        .map(|r| {
            if !r.id.starts_with("wild_trail_") {
                return None;
            }
            let start = *r.points.first()?;
            let (word, _) = ends.iter().min_by(|a, b| dist2(a.1, start).total_cmp(&dist2(b.1, start)))?;
            Some(format!("{word} Trail"))
        })
        .collect();
    for (road, name) in roads.iter_mut().zip(trail_names) {
        if let Some(name) = name {
            road.name = Some(names.take(name));
        }
    }

    // The passes and the climbs
    let mut rng = SplitMix64::stream(seed, &[SALT, 1]);
    let mut upland = Draw::new(&UPLAND, &mut rng);
    for road in roads.iter_mut() {
        let suffix = if road.id.starts_with("wild_pass_") {
            "Edge"
        } else if road.id.starts_with("wild_climb_") {
            "Rake"
        } else {
            continue;
        };
        road.name = Some(upland.next(&mut names, &mut rng, |w| format!("{w} {suffix}")));
    }

    // The urban grid: its ring streets, then its spokes
    let mut rng = SplitMix64::stream(seed, &[SALT, 2]);
    let mut trades = Draw::new(&TRADES, &mut rng);
    for road in roads.iter_mut().filter(|r| r.id.starts_with("urb_ring_") && r.name.is_none()) {
        road.name = Some(trades.next(&mut names, &mut rng, |w| format!("{w} Street")));
    }
    for road in roads.iter_mut().filter(|r| r.id.starts_with("urb_spoke_")) {
        road.name = Some(trades.next(&mut names, &mut rng, |w| format!("{w}gate")));
    }

    // The suburb, an estate to a neighbourhood
    let mut rng = SplitMix64::stream(seed, &[SALT, 3]);
    let themes = shuffled(&ESTATES, &mut rng);
    let mut estates: Vec<(u64, u64)> = Vec::new();
    for road in roads.iter() {
        if let Some((_, s, n)) = suburban(&road.id) {
            if !estates.contains(&(s, n)) {
                estates.push((s, n));
            }
        }
    }
    for (k, &(s, n)) in estates.iter().enumerate() {
        let mut rng = SplitMix64::stream(seed, &[SALT, 3, s, n]);
        let mut words = Draw::new(themes[k % themes.len()], &mut rng);
        for road in roads.iter_mut() {
            let Some((kind, rs, rn)) = suburban(&road.id) else { continue };
            if (rs, rn) != (s, n) {
                continue;
            }
            let suffixes: &[&str] = match kind {
                "collector" if turning(&road.points) > 40.0 => &["Crescent", "Drive", "Drive", "Way"],
                "collector" => &["Avenue", "Road", "Gardens", "Grove"],
                "culdesac" => &["Close", "Close", "Close", "Court", "Court", "Place", "Mews", "End"],
                "loop" => &["Circle", "Crescent"],
                "entrance" => &["Rise", "Way", "Approach"],
                _ => &["Walk", "Walk", "Path", "Row"],
            };
            let suffix = suffixes[rng.next_u64() as usize % suffixes.len()];
            road.name = Some(words.next(&mut names, &mut rng, |w| format!("{w} {suffix}")));
        }
    }

    // The rural ring, by its sectors' characters
    let mut rng = SplitMix64::stream(seed, &[SALT, 4]);
    let mut settlers = Draw::new(&SETTLERS, &mut rng);
    let mut lanes = Draw::new(&LANES, &mut rng);
    for road in roads.iter_mut() {
        let Some((s, k)) = lane(&road.id) else { continue };
        // The plan numbers a sector's main roads first (`rural::Plan::roads`)
        let main = k < mains;
        let character = rural.iter().find(|x| x.index == s).map_or(Character::Winding, |x| x.character);
        road.name = Some(match character {
            Character::Concession => {
                let suffix = if main { "Line" } else { "Sideroad" };
                settlers.next(&mut names, &mut rng, |w| format!("{w} {suffix}"))
            }
            Character::Winding => {
                let suffix = if main { "Lane" } else { ["Lane", "Drove", "Track"][rng.next_u64() as usize % 3] };
                lanes.next(&mut names, &mut rng, |w| format!("{w} {suffix}"))
            }
        });
    }
}

/// `lane_{s}_{k}`'s sector and number
fn lane(id: &str) -> Option<(usize, usize)> {
    let mut parts = id.strip_prefix("lane_")?.split('_');
    let s = parts.next()?.parse().ok()?;
    let k = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((s, k))
}

/// A suburban street's kind, sector and neighbourhood, from its id
/// (`collector_{s}_{n}_…`, `culdesac_…`, `entrance_…`, `link_…`, `loop_…`)
fn suburban(id: &str) -> Option<(&str, u64, u64)> {
    let (kind, rest) = id.split_once('_')?;
    if !matches!(kind, "collector" | "culdesac" | "entrance" | "link" | "loop") {
        return None;
    }
    let mut parts = rest.split('_');
    Some((kind, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// How far a road turns along its length, in degrees
fn turning(points: &[P]) -> f64 {
    points
        .windows(3)
        .map(|w| {
            let (a, b) = ((w[1].0 - w[0].0, w[1].1 - w[0].1), (w[2].0 - w[1].0, w[2].1 - w[1].1));
            let cross = a.0 * b.1 - a.1 * b.0;
            let dot = a.0 * b.0 + a.1 * b.1;
            use capitol_protocol::fmath::StableMath;
            cross.stable_atan2(dot).abs().to_degrees()
        })
        .sum()
}

fn dist2(a: P, b: P) -> f64 {
    (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)
}

/// `items`, shuffled by `rng`
fn shuffled<T: Copy>(items: &[T], rng: &mut SplitMix64) -> Vec<T> {
    let mut out = items.to_vec();
    for i in (1..out.len()).rev() {
        out.swap(i, rng.next_u64() as usize % (i + 1));
    }
    out
}

/// The names given so far
#[derive(Default)]
struct Names {
    used: HashSet<String>,
}

impl Names {
    /// `name`, or the first of its elder forms ("Old Mill Lane") not given yet
    fn take(&mut self, name: String) -> String {
        let name = std::iter::once(name.clone())
            .chain(ELDER.iter().map(|e| format!("{e} {name}")))
            .chain((2..).map(|n| format!("{name} {n}")))
            .find(|n| !self.used.contains(n))
            .expect("an endless supply");
        self.used.insert(name.clone());
        name
    }
}

/// A table's words in a shuffled order, each given once, and then round
/// again in a fresh order should a family need more
struct Draw<'a> {
    table: &'a [&'a str],
    order: Vec<&'a str>,
}

impl<'a> Draw<'a> {
    fn new(table: &'a [&'a str], rng: &mut SplitMix64) -> Self {
        Self { table, order: shuffled(table, rng) }
    }

    /// The next word, formed by `form`, that no road has yet; elder forms
    /// once the table's been round
    fn next(&mut self, names: &mut Names, rng: &mut SplitMix64, form: impl Fn(&str) -> String) -> String {
        loop {
            if self.order.is_empty() {
                // Round again: every form of every word is taken, so
                // `take` gives the elder forms
                let word = self.table[rng.next_u64() as usize % self.table.len()];
                return names.take(form(word));
            }
            let name = form(self.order.remove(0));
            if !names.used.contains(&name) {
                return names.take(name);
            }
        }
    }
}

/// Each lot's address: lot id to "7 Larch Close", or on a ring street, with
/// its nearest crossing, "112 Mercer Street by Tannergate". Its number counts
/// along its street from the street's start, odd on the left and even on the
/// right, as a street's houses are numbered. A lot whose street has no name
/// is left out.
pub fn addresses(roads: &[RoadSegment], lots: &[TerrainPlot]) -> HashMap<String, String> {
    let by_id: HashMap<&str, &RoadSegment> = roads.iter().map(|r| (r.id.as_str(), r)).collect();
    let crossings: Vec<&RoadSegment> = roads
        .iter()
        .filter(|r| r.name.is_some())
        .filter(|r| r.id.starts_with("urb_spoke_") || (r.id.starts_with("arterial_") && r.id != "arterial_capitol"))
        .collect();
    // Each street's lots, by side and how far along it their fronts are
    let mut along: HashMap<(&str, bool), Vec<(f64, &str)>> = HashMap::new();
    for lot in lots {
        let Some(road) = by_id.get(lot.adjacent_street_id.as_str()).filter(|r| r.name.is_some()) else { continue };
        let (a, b) = (lot.corners[lot.street_facing_edge.0], lot.corners[lot.street_facing_edge.1]);
        let (s, left) = station(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), &road.points);
        along.entry((road.id.as_str(), left)).or_default().push((s, lot.id.as_str()));
    }
    let mut numbers: HashMap<&str, usize> = HashMap::new();
    for ((_, left), mut side) in along {
        side.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        for (k, (_, id)) in side.into_iter().enumerate() {
            numbers.insert(id, 2 * k + if left { 1 } else { 2 });
        }
    }
    lots.iter()
        .filter_map(|lot| {
            let road = by_id.get(lot.adjacent_street_id.as_str())?;
            let street = format!("{} {}", numbers.get(lot.id.as_str())?, road.name.as_deref()?);
            if !is_ring(&road.id) {
                return Some((lot.id.clone(), street));
            }
            let middle = lot.corners.iter().fold((0.0, 0.0), |m, c| (m.0 + c.0 / 4.0, m.1 + c.1 / 4.0));
            let near = crossings
                .iter()
                .map(|r| (r, distance_to(middle, &r.points)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .and_then(|(r, _)| r.name.as_deref());
            Some((lot.id.clone(), near.map_or(street.clone(), |n| format!("{street} by {n}"))))
        })
        .collect()
}

/// How far along `points` the point nearest `p` is, and whether `p` is on
/// its left, going along it (y points south, so the left is a turn the
/// other way from the maths')
fn station(p: P, points: &[P]) -> (f64, bool) {
    let mut best = (f64::INFINITY, 0.0, true);
    let mut start = 0.0;
    for w in points.windows(2) {
        let (d, len) = ((w[1].0 - w[0].0, w[1].1 - w[0].1), dist2(w[0], w[1]).sqrt());
        let t = if len > 0.0 { (((p.0 - w[0].0) * d.0 + (p.1 - w[0].1) * d.1) / (len * len)).clamp(0.0, 1.0) } else { 0.0 };
        let at = (w[0].0 + d.0 * t, w[0].1 + d.1 * t);
        let gap = dist2(p, at);
        if gap < best.0 {
            let cross = d.0 * (p.1 - w[0].1) - d.1 * (p.0 - w[0].0);
            best = (gap, start + len * t, cross < 0.0);
        }
        start += len;
    }
    (best.1, best.2)
}

/// A street that runs round the tower, where a lot wants its crossing too
fn is_ring(id: &str) -> bool {
    id.starts_with("urb_ring_") || id == "guild_ring" || id == "trade_ring"
}

fn distance_to(p: P, points: &[P]) -> f64 {
    points
        .windows(2)
        .map(|w| super::terrain::point_to_segment_distance(p.0, p.1, w[0].0, w[0].1, w[1].0, w[1].1))
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TerrainConfig, WorldGenParams};

    #[test]
    fn every_road_a_lot_fronts_has_a_name() {
        let world = TerrainConfig::default_world();
        let named: HashMap<&str, &str> =
            world.roads.iter().filter_map(|r| Some((r.id.as_str(), r.name.as_deref()?))).collect();
        let nameless: Vec<(&str, &str)> = world
            .terrain_plots
            .iter()
            .filter(|l| !named.contains_key(l.adjacent_street_id.as_str()))
            .map(|l| (l.id.as_str(), l.adjacent_street_id.as_str()))
            .collect();
        assert!(nameless.is_empty(), "{} lots front a road with no name: {:?}", nameless.len(), &nameless[..nameless.len().min(10)]);
        // And so every lot has an address
        assert_eq!(world.addresses().len(), world.terrain_plots.len());
    }

    #[test]
    fn no_two_streets_share_a_name() {
        // But a great road's stretches, either side of the canal
        let world = TerrainConfig::default_world();
        let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
        for r in &world.roads {
            if let Some(name) = r.name.as_deref() {
                by_name.entry(name).or_default().push(&r.id);
            }
        }
        for (name, ids) in by_name {
            let great = ids.iter().all(|id| id.starts_with("arterial_"));
            assert!(ids.len() == 1 || great, "{name}: {ids:?}");
        }
    }

    #[test]
    fn every_road_but_the_wild_spurs_is_named() {
        let world = TerrainConfig::default_world();
        let nameless: Vec<&str> =
            world.roads.iter().filter(|r| r.name.is_none() && !r.id.starts_with("wild_spur_")).map(|r| r.id.as_str()).collect();
        assert!(nameless.is_empty(), "{nameless:?}");
    }

    #[test]
    fn the_same_seed_names_the_world_alike_and_another_seed_otherwise() {
        let world = TerrainConfig::default_world();
        let params = WorldGenParams::default();
        let sectors = crate::models::rural::sectors_of(&params, (world.center_x, world.center_y), &world.roads);
        let name = |seed| {
            let mut roads = world.roads.clone();
            roads.iter_mut().for_each(|r| r.name = None);
            name_roads(&mut roads, seed, &sectors, params.rural.mains.len());
            roads.into_iter().map(|r| r.name).collect::<Vec<_>>()
        };
        let again = name(params.seed);
        assert_eq!(again, world.roads.iter().map(|r| r.name.clone()).collect::<Vec<_>>());
        let other = name(params.seed + 1);
        let differ = again.iter().zip(&other).filter(|(a, b)| a != b).count();
        assert!(differ > again.len() / 2, "only {differ} of {} roads renamed", again.len());
    }

    #[test]
    fn the_road_in_is_the_pilgrims_road_whatever_the_seed() {
        let world = TerrainConfig::default_world();
        let params = WorldGenParams::default();
        let sectors = crate::models::rural::sectors_of(&params, (world.center_x, world.center_y), &world.roads);
        for seed in [params.seed, 1, 7, 1234] {
            let mut roads = world.roads.clone();
            name_roads(&mut roads, seed, &sectors, params.rural.mains.len());
            let name = |id: &str| roads.iter().find(|r| r.id == id).and_then(|r| r.name.clone());
            // arterial_4 runs south, and Capitol Avenue goes on from it
            assert_eq!(name("arterial_4").as_deref(), Some("Pilgrims' Road"), "seed {seed}");
            assert_eq!(name("arterial_inner_4"), name("arterial_4"));
            let greats: HashSet<String> = (0..8).filter_map(|i| name(&format!("arterial_{i}"))).collect();
            assert_eq!(greats.len(), 8, "seed {seed}: {greats:?}");
        }
    }

    #[test]
    fn every_lot_has_an_address_of_its_own() {
        let world = TerrainConfig::default_world();
        let addresses = world.addresses();
        let mut seen: HashMap<&str, &str> = HashMap::new();
        for (lot, address) in &addresses {
            // Without the crossing, which only helps find it
            let address = address.split(" by ").next().expect("an address");
            if let Some(other) = seen.insert(address, lot) {
                panic!("{lot} and {other} are both {address}");
            }
        }
        // Odd on the left and even on the right, from 1 and 2
        let on_street = |street: &str| {
            let mut n: Vec<usize> = addresses
                .values()
                .filter_map(|a| a.strip_suffix(street)?.trim().parse().ok())
                .collect();
            n.sort();
            n
        };
        let lot = &world.terrain_plots.iter().find(|l| l.zone == "rural").expect("a farm").adjacent_street_id;
        let street = world.roads.iter().find(|r| &r.id == lot).and_then(|r| r.name.clone()).expect("named");
        let numbers = on_street(&format!(" {street}"));
        assert!(numbers.contains(&1) || numbers.contains(&2), "{street}: {numbers:?}");
        let (odd, even): (Vec<usize>, Vec<usize>) = numbers.iter().partition(|n| *n % 2 == 1);
        assert!(odd.iter().enumerate().all(|(k, n)| *n == 2 * k + 1), "{street}: {odd:?}");
        assert!(even.iter().enumerate().all(|(k, n)| *n == 2 * k + 2), "{street}: {even:?}");
    }

    #[test]
    fn a_ring_street_lot_is_placed_by_its_crossing() {
        let world = TerrainConfig::default_world();
        let addresses = world.addresses();
        let urban = world.terrain_plots.iter().find(|l| l.adjacent_street_id.starts_with("urb_ring_")).expect("an urban lot");
        let address = &addresses[&urban.id];
        assert!(address.contains(" by "), "{address}");
        let suburban = world.terrain_plots.iter().find(|l| l.zone == "suburban").expect("a house");
        assert!(!addresses[&suburban.id].contains(" by "), "{}", addresses[&suburban.id]);
    }

    /// Every road's name, by family, and some addresses: for a look at them
    #[test]
    #[ignore]
    fn print_names() {
        let world = TerrainConfig::default_world();
        for r in &world.roads {
            println!("{}\t{}", r.id, r.name.as_deref().unwrap_or("-"));
        }
        let addresses = world.addresses();
        let hall = world.landmarks[0].door;
        for zone in ["rural", "suburban", "urban"] {
            let mut near: Vec<_> = world.terrain_plots.iter().filter(|l| l.zone == zone).collect();
            let d = |l: &TerrainPlot| dist2(l.corners[0], hall);
            near.sort_by(|a, b| d(a).total_cmp(&d(b)));
            for lot in near.iter().take(12) {
                println!("NEAR	{zone}	{}	{}", lot.id, addresses[&lot.id]);
            }
        }
        for lot in world.terrain_plots.iter().step_by(97) {
            println!("ADDR\t{}\t{}\t{}", lot.zone, lot.id, addresses.get(&lot.id).map_or("-", |a| a.as_str()));
        }
    }
}

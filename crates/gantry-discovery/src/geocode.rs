//! Offline geocoding against bundled GeoNames data (CC BY 4.0, see
//! `data/geo/README.md`). No geocoding service is called, so neither the
//! user's address nor posting locations leave the machine (§12).
//!
//! Coverage: populated places with population ≥ 5,000 worldwide, US ZIP
//! code centroids, admin-1 regions and countries. Anything else resolves
//! to `unresolved` and the posting is flagged, never guessed.

use std::collections::HashMap;
use std::sync::OnceLock;

use gantry_core::geo::Place;
use gantry_core::location::{self, Area, Location, ParsedLocation};
use gantry_core::text::normalize;

const CITIES: &str = include_str!("../../../data/geo/cities.tsv");
const POSTAL_US: &str = include_str!("../../../data/geo/postal_us.tsv");
const ADMIN1: &str = include_str!("../../../data/geo/admin1.tsv");
const COUNTRIES: &str = include_str!("../../../data/geo/countries.tsv");

/// A city whose name equals a region or country name wins over the region
/// only above this population ("New York" the city, "California" the state).
const CITY_OVER_REGION_POPULATION: u32 = 1_000_000;

/// Country names used in postings that GeoNames spells differently.
const COUNTRY_ALIASES: &[(&str, &str)] = &[
    ("usa", "US"),
    ("u s a", "US"),
    ("u s", "US"),
    ("united states of america", "US"),
    ("america", "US"),
    ("uk", "GB"),
    ("u k", "GB"),
    ("great britain", "GB"),
    ("britain", "GB"),
    ("england", "GB"),
    ("scotland", "GB"),
    ("wales", "GB"),
    ("northern ireland", "GB"),
    ("uae", "AE"),
    ("korea", "KR"),
    ("republic of korea", "KR"),
    ("czech republic", "CZ"),
    ("holland", "NL"),
    ("the netherlands", "NL"),
    ("viet nam", "VN"),
    ("turkiye", "TR"),
];

/// Region abbreviations that are not GeoNames admin-1 codes:
/// `(country, abbreviation, GeoNames admin-1 name)`.
const REGION_ABBREVIATIONS: &[(&str, &str, &str)] = &[
    ("CA", "AB", "Alberta"),
    ("CA", "BC", "British Columbia"),
    ("CA", "MB", "Manitoba"),
    ("CA", "NB", "New Brunswick"),
    ("CA", "NL", "Newfoundland and Labrador"),
    ("CA", "NS", "Nova Scotia"),
    ("CA", "NT", "Northwest Territories"),
    ("CA", "NU", "Nunavut"),
    ("CA", "ON", "Ontario"),
    ("CA", "PE", "Prince Edward Island"),
    ("CA", "QC", "Quebec"),
    ("CA", "SK", "Saskatchewan"),
    ("CA", "YT", "Yukon"),
    ("AU", "ACT", "Australian Capital Territory"),
    ("AU", "NSW", "New South Wales"),
    ("AU", "NT", "Northern Territory"),
    ("AU", "QLD", "Queensland"),
    ("AU", "SA", "South Australia"),
    ("AU", "TAS", "Tasmania"),
    ("AU", "VIC", "Victoria"),
    ("AU", "WA", "Western Australia"),
];

/// Informal place names that are not in GeoNames: `(name, city, admin-1)`,
/// resolved to that city in the US.
const PLACE_ALIASES: &[(&str, &str, &str)] = &[
    ("bay area", "San Francisco", "CA"),
    ("sf bay area", "San Francisco", "CA"),
    ("san francisco bay area", "San Francisco", "CA"),
    ("silicon valley", "San Jose", "CA"),
    ("dc", "Washington", "DC"),
    ("washington dc", "Washington", "DC"),
    ("washington d c", "Washington", "DC"),
];

const CONTINENTS: &[(&str, &str)] = &[
    ("europe", "EU"),
    ("asia", "AS"),
    ("africa", "AF"),
    ("oceania", "OC"),
    ("north america", "NA"),
    ("south america", "SA"),
];

#[derive(Debug)]
struct City {
    name: String,
    country: String,
    admin1: String,
    lat: f64,
    lon: f64,
    population: u32,
}

#[derive(Debug)]
pub struct Gazetteer {
    cities: Vec<City>,
    by_name: HashMap<String, Vec<usize>>,
    by_alt: HashMap<String, Vec<usize>>,
    /// Normalized country name, ISO2 or ISO3 → ISO2.
    countries: HashMap<String, String>,
    continent_of: HashMap<String, String>,
    /// (ISO2, normalized admin-1 name or code) → admin-1 code.
    admin1: HashMap<(String, String), String>,
    /// Normalized admin-1 name → (ISO2, code), for names standing alone.
    admin1_names: HashMap<String, Vec<(String, String)>>,
    /// US ZIP → (place name, state, lat, lon).
    postal_us: HashMap<String, (String, String, f64, f64)>,
    /// (state, normalized place name) → ZIP, for US places below the
    /// city population cutoff ("Annapolis Junction, MD").
    postal_places: HashMap<(String, String), String>,
}

/// The bundled gazetteer, loaded on first use.
pub fn gazetteer() -> &'static Gazetteer {
    static GAZ: OnceLock<Gazetteer> = OnceLock::new();
    GAZ.get_or_init(Gazetteer::load)
}

impl Gazetteer {
    fn load() -> Self {
        let mut g = Self {
            cities: Vec::new(),
            by_name: HashMap::new(),
            by_alt: HashMap::new(),
            countries: HashMap::new(),
            continent_of: HashMap::new(),
            admin1: HashMap::new(),
            admin1_names: HashMap::new(),
            postal_us: HashMap::new(),
            postal_places: HashMap::new(),
        };
        for line in COUNTRIES.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [iso2, iso3, name, continent] = f[..] else {
                continue;
            };
            g.countries.insert(normalize(iso2), iso2.to_owned());
            g.countries.insert(normalize(iso3), iso2.to_owned());
            g.countries.insert(normalize(name), iso2.to_owned());
            g.continent_of.insert(iso2.to_owned(), continent.to_owned());
        }
        for (alias, iso2) in COUNTRY_ALIASES {
            g.countries.insert((*alias).to_owned(), (*iso2).to_owned());
        }
        for line in ADMIN1.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [country, code, name] = f[..] else {
                continue;
            };
            let key = (country.to_owned(), normalize(name));
            g.admin1.insert(key, code.to_owned());
            g.admin1
                .insert((country.to_owned(), normalize(code)), code.to_owned());
            g.admin1_names
                .entry(normalize(name))
                .or_default()
                .push((country.to_owned(), code.to_owned()));
        }
        for (country, abbr, name) in REGION_ABBREVIATIONS {
            if let Some(code) = g
                .admin1
                .get(&((*country).to_owned(), normalize(name)))
                .cloned()
            {
                g.admin1
                    .insert(((*country).to_owned(), normalize(abbr)), code);
            }
        }
        for line in CITIES.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [name, ascii, alts, lat, lon, country, admin1, population] = f[..] else {
                continue;
            };
            let (Ok(lat), Ok(lon)) = (lat.parse(), lon.parse()) else {
                continue;
            };
            let idx = g.cities.len();
            g.cities.push(City {
                name: name.to_owned(),
                country: country.to_owned(),
                admin1: admin1.to_owned(),
                lat,
                lon,
                population: population.parse().unwrap_or(0),
            });
            let mut names = vec![normalize(name)];
            let ascii = normalize(ascii);
            if !names.contains(&ascii) {
                names.push(ascii);
            }
            for n in names {
                g.by_name.entry(n).or_default().push(idx);
            }
            for alt in alts.split(',').filter(|a| !a.is_empty()) {
                g.by_alt.entry(normalize(alt)).or_default().push(idx);
            }
        }
        for line in POSTAL_US.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [_, zip, place, state, lat, lon] = f[..] else {
                continue;
            };
            if let (Ok(lat), Ok(lon)) = (lat.parse(), lon.parse()) {
                g.postal_places
                    .entry((state.to_owned(), normalize(place)))
                    .or_insert_with(|| zip.to_owned());
                g.postal_us.insert(
                    zip.to_owned(),
                    (place.to_owned(), state.to_owned(), lat, lon),
                );
            }
        }
        g
    }

    fn place(&self, idx: usize) -> Place {
        let c = &self.cities[idx];
        Place {
            name: c.name.clone(),
            admin1: (!c.admin1.is_empty()).then(|| c.admin1.clone()),
            country: c.country.clone(),
            lat: c.lat,
            lon: c.lon,
        }
    }

    fn country(&self, text: &str) -> Option<&str> {
        self.countries.get(&normalize(text)).map(String::as_str)
    }

    fn continent(text: &str) -> Option<&'static str> {
        let n = normalize(text);
        CONTINENTS
            .iter()
            .find(|(name, _)| *name == n)
            .map(|(_, c)| *c)
    }

    /// The admin-1 code `text` names within `country`, by code or name.
    fn admin1_in(&self, country: &str, text: &str) -> Option<&str> {
        self.admin1
            .get(&(country.to_owned(), normalize(text)))
            .map(String::as_str)
    }

    /// A region named on its own ("California"), preferring the US when
    /// the name exists in several countries.
    fn lone_admin1(&self, text: &str) -> Option<(String, String)> {
        let found = self.admin1_names.get(&normalize(text))?;
        found
            .iter()
            .find(|(c, _)| c == "US")
            .or_else(|| (found.len() == 1).then(|| &found[0]))
            .cloned()
    }

    fn city_candidates(&self, text: &str) -> Vec<usize> {
        let n = expand_abbreviations(&normalize(text));
        if let Some(found) = self.by_name.get(&n) {
            return found.clone();
        }
        self.by_alt.get(&n).cloned().unwrap_or_default()
    }

    /// A US place too small for the city list, found by name among ZIP
    /// code places in the given state.
    fn postal_place(&self, text: &str, state: &str) -> Option<Place> {
        let key = (state.to_owned(), expand_abbreviations(&normalize(text)));
        self.postal_places
            .get(&key)
            .and_then(|zip| self.postal_us(zip))
    }

    /// The area a qualifier names on its own: a country, a US state code
    /// or name, or a Canadian or Australian region abbreviation.
    fn qualifier_area(&self, text: &str) -> Option<Area> {
        if let Some(country) = self.country(text) {
            return Some(Area::Country {
                country: country.to_owned(),
            });
        }
        for country in ["US", "CA", "AU"] {
            if let Some(code) = self.admin1_in(country, text) {
                return Some(Area::Admin1 {
                    country: country.to_owned(),
                    admin1: code.to_owned(),
                });
            }
        }
        self.lone_admin1(text)
            .map(|(country, admin1)| Area::Admin1 { country, admin1 })
    }

    fn biggest(&self, candidates: impl Iterator<Item = usize>) -> Option<usize> {
        candidates.max_by_key(|i| self.cities[*i].population)
    }

    pub fn postal_us(&self, zip: &str) -> Option<Place> {
        self.postal_us
            .get(zip)
            .map(|(name, state, lat, lon)| Place {
                name: name.clone(),
                admin1: Some(state.clone()),
                country: "US".into(),
                lat: *lat,
                lon: *lon,
            })
    }

    pub fn continent_of(&self, country: &str) -> Option<&str> {
        self.continent_of.get(country).map(String::as_str)
    }

    /// Resolves one parsed location entry. A run-together list
    /// ("San Francisco, CA, Seattle WA") yields one location per place.
    pub fn resolve(&self, parsed: &ParsedLocation) -> Vec<Location> {
        let make =
            |place: Option<Place>, area: Option<Area>, unresolved: Option<String>| Location {
                raw: parsed.raw.clone(),
                modes: parsed.modes.clone(),
                place,
                area,
                unresolved,
            };
        if parsed.parts.is_empty() {
            return if parsed.anywhere {
                vec![make(None, Some(Area::Anywhere), None)]
            } else if parsed.modes.is_empty() {
                Vec::new()
            } else {
                vec![make(None, None, None)]
            };
        }
        let parts = &parsed.parts;
        let mut out = Vec::new();
        let mut i = 0;
        while i < parts.len() {
            let (consumed, place, area) = self.resolve_at(parts, i);
            if consumed == 0 {
                // "Starbase, TX": the place is unknown but its region is
                // not; keep both together, flagged.
                let qualifier = parts.get(i + 1).and_then(|q| self.qualifier_area(q));
                if let Some(area) = qualifier {
                    out.push(make(
                        None,
                        Some(area),
                        Some(format!("{}, {}", parts[i], parts[i + 1])),
                    ));
                    i += 2;
                } else {
                    out.push(make(None, None, Some(parts[i].clone())));
                    i += 1;
                }
            } else {
                out.push(make(place, area, None));
                i += consumed;
            }
        }
        out
    }

    /// Resolves the place starting at `parts[i]`. Returns how many parts it
    /// used (0 if none matched) and the place or area found.
    fn resolve_at(&self, parts: &[String], i: usize) -> (usize, Option<Place>, Option<Area>) {
        let part = parts[i].as_str();
        let next = parts.get(i + 1).map(String::as_str);
        let after = parts.get(i + 2).map(String::as_str);

        if is_us_zip(part)
            && let Some(p) = self.postal_us(part)
        {
            return (1, Some(p), None);
        }
        if let Some(code) = Self::continent(part) {
            return (1, None, Some(Area::Continent { code: code.into() }));
        }
        // "US-CA-Menlo Park", "GB-London": ISO country, optional region,
        // then the place.
        if let Some((head, rest)) = part.split_once('-')
            && head.len() == 2
            && head.bytes().all(|b| b.is_ascii_uppercase())
            && self.country(head) == Some(head)
        {
            let expanded: Vec<String> = match rest.split_once('-') {
                Some((region, city)) if self.admin1_in(head, region).is_some() => {
                    vec![city.to_owned(), region.to_owned(), head.to_owned()]
                }
                _ => vec![rest.to_owned(), head.to_owned()],
            };
            let (used, place, area) = self.resolve_at(&expanded, 0);
            if used > 0 {
                return (1, place, area);
            }
        }
        let n = normalize(part);
        if let Some((_, city, admin1)) = PLACE_ALIASES.iter().find(|(alias, ..)| *alias == n) {
            let idx =
                self.biggest(self.city_candidates(city).into_iter().filter(|c| {
                    self.cities[*c].country == "US" && self.cities[*c].admin1 == *admin1
                }));
            if let Some(idx) = idx {
                let used = if next.is_some_and(|q| self.admin1_in("US", q) == Some(admin1)) {
                    2
                } else {
                    1
                };
                return (used, Some(self.place(idx)), None);
            }
        }

        let candidates = self.city_candidates(part);
        // "City, Region, Country" and "City, Region|Country".
        if let Some(q1) = next {
            let with_admin: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|c| {
                    let city = &self.cities[*c];
                    self.admin1_in(&city.country, q1) == Some(city.admin1.as_str())
                })
                .collect();
            if !with_admin.is_empty() {
                let in_country: Vec<usize> = match after.and_then(|q2| self.country(q2)) {
                    Some(country) => with_admin
                        .iter()
                        .copied()
                        .filter(|c| self.cities[*c].country == country)
                        .collect(),
                    None => Vec::new(),
                };
                if !in_country.is_empty() {
                    return (
                        3,
                        self.biggest(in_country.into_iter()).map(|c| self.place(c)),
                        None,
                    );
                }
                return (
                    2,
                    self.biggest(with_admin.into_iter()).map(|c| self.place(c)),
                    None,
                );
            }
            if let Some(state) = self.admin1_in("US", q1)
                && let Some(place) = self.postal_place(part, state)
            {
                return (2, Some(place), None);
            }
            if let Some(country) = self.country(q1) {
                let in_country = candidates
                    .iter()
                    .copied()
                    .filter(|c| self.cities[*c].country == country);
                if let Some(c) = self.biggest(in_country) {
                    return (2, Some(self.place(c)), None);
                }
            }
        }

        let biggest_city = self.biggest(candidates.iter().copied());
        if let Some(country) = self.country(part) {
            let city_wins = biggest_city.is_some_and(|c| {
                let city = &self.cities[c];
                city.country == country || city.population >= CITY_OVER_REGION_POPULATION
            });
            if !city_wins {
                return (
                    1,
                    None,
                    Some(Area::Country {
                        country: country.to_owned(),
                    }),
                );
            }
        }
        if let Some((country, code)) = self.lone_admin1(part) {
            let city_wins = biggest_city
                .is_some_and(|c| self.cities[c].population >= CITY_OVER_REGION_POPULATION);
            if !city_wins {
                let used = if next.is_some_and(|q| self.country(q) == Some(country.as_str())) {
                    2
                } else {
                    1
                };
                return (
                    used,
                    None,
                    Some(Area::Admin1 {
                        country,
                        admin1: code,
                    }),
                );
            }
        }
        if let Some(c) = biggest_city {
            return (1, Some(self.place(c)), None);
        }
        // A region code standing alone: "TX", "ON".
        if part.len() <= 3
            && part.bytes().all(|b| b.is_ascii_uppercase())
            && let Some(area) = self.qualifier_area(part)
        {
            return (1, None, Some(area));
        }
        // "Seattle WA": a trailing region code inside the part.
        if let Some((city, region)) = part.rsplit_once(' ') {
            let split = [city.to_owned(), region.to_owned()];
            let (used, place, area) = self.resolve_at(&split, 0);
            if used == 2 {
                return (1, place, area);
            }
        }
        (0, None, None)
    }

    /// Geocodes a user-entered place: a US ZIP code (alone or at the end)
    /// or "City, Region".
    pub fn locate(&self, text: &str) -> Option<Place> {
        if let Some(zip) = text
            .split(|c: char| !c.is_ascii_digit())
            .rev()
            .find(|t| is_us_zip(t))
            && let Some(p) = self.postal_us(zip)
        {
            return Some(p);
        }
        location::parse(text)
            .iter()
            .flat_map(|p| self.resolve(p))
            .find_map(|l| l.place)
    }
}

/// "ft meade" → "fort meade", "mt view" → "mount view"; GeoNames spells
/// these out.
fn expand_abbreviations(normalized: &str) -> String {
    match normalized.split_once(' ') {
        Some(("ft", rest)) => format!("fort {rest}"),
        Some(("mt", rest)) => format!("mount {rest}"),
        Some(("pt", rest)) => format!("point {rest}"),
        _ => normalized.to_owned(),
    }
}

fn is_us_zip(s: &str) -> bool {
    s.len() == 5 && s.bytes().all(|b| b.is_ascii_digit())
}

/// Parses and geocodes every raw location string of a posting.
pub fn locate_all(raws: &[String]) -> Vec<Location> {
    let gaz = gazetteer();
    raws.iter()
        .flat_map(|raw| location::parse(raw))
        .flat_map(|p| gaz.resolve(&p))
        .collect()
}

#[cfg(test)]
mod tests {
    use gantry_core::WorkMode;

    use super::*;

    fn one(raw: &str) -> Location {
        let mut v = locate_all(&[raw.to_owned()]);
        assert_eq!(v.len(), 1, "{raw}: {v:#?}");
        v.remove(0)
    }

    fn place(raw: &str) -> Place {
        one(raw)
            .place
            .unwrap_or_else(|| panic!("no place for {raw}"))
    }

    #[test]
    fn us_city_state_forms() {
        let p = place("San Francisco, CA");
        assert_eq!(
            (p.country.as_str(), p.admin1.as_deref()),
            ("US", Some("CA"))
        );
        assert_eq!(place("Austin, Texas").admin1.as_deref(), Some("TX"));
        assert_eq!(place("Washington, DC").admin1.as_deref(), Some("DC"));
        assert_eq!(place("New York, NY").name, "New York City");
        assert_eq!(place("NYC").name, "New York City");
        assert_eq!(place("Portland, OR").admin1.as_deref(), Some("OR"));
        assert_eq!(place("Portland, ME").admin1.as_deref(), Some("ME"));
    }

    #[test]
    fn international_forms() {
        assert_eq!(place("London, United Kingdom").country, "GB");
        assert_eq!(place("London").country, "GB", "largest London");
        assert_eq!(place("London, ON").country, "CA");
        assert_eq!(place("Bangalore, India").name, "Bengaluru");
        assert_eq!(place("Berlin, Germany").country, "DE");
        assert_eq!(place("São Paulo").country, "BR");
        assert_eq!(place("Singapore").country, "SG");
        assert_eq!(place("Sydney, NSW").country, "AU");
    }

    #[test]
    fn remote_areas() {
        let l = one("Remote - US");
        assert_eq!(l.modes, [WorkMode::Remote]);
        assert_eq!(
            l.area,
            Some(Area::Country {
                country: "US".into()
            })
        );
        assert_eq!(
            one("Remote, USA").area,
            Some(Area::Country {
                country: "US".into()
            })
        );
        assert_eq!(
            one("Remote - California").area,
            Some(Area::Admin1 {
                country: "US".into(),
                admin1: "CA".into()
            })
        );
        assert_eq!(
            one("Remote (Europe)").area,
            Some(Area::Continent { code: "EU".into() })
        );
        assert_eq!(one("REMOTE (Worldwide)").area, Some(Area::Anywhere));
        let bare = one("Remote");
        assert_eq!((bare.place, bare.area, bare.unresolved), (None, None, None));
    }

    #[test]
    fn run_together_list_and_trailing_state() {
        let v = locate_all(&["San Francisco, CA, Seattle WA, New York, NY".to_owned()]);
        let names: Vec<_> = v
            .iter()
            .filter_map(|l| l.place.as_ref())
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["San Francisco", "Seattle", "New York City"]);
    }

    #[test]
    fn small_us_places_from_zip_data() {
        let p = place("Annapolis Junction, MD");
        assert_eq!(
            (p.country.as_str(), p.admin1.as_deref()),
            ("US", Some("MD"))
        );
        assert_eq!(place("Linthicum Heights, MD").admin1.as_deref(), Some("MD"));
        assert_eq!(place("Ft. Meade, MD").admin1.as_deref(), Some("MD"));
    }

    #[test]
    fn iso_prefixed_and_site_forms() {
        let p = place("US-CA-Menlo Park");
        assert_eq!(
            (p.name.as_str(), p.admin1.as_deref()),
            ("Menlo Park", Some("CA"))
        );
        assert_eq!(place("GB-London").country, "GB");
        assert_eq!(place("US-NY-New York").name, "New York City");
        assert_eq!(place("San Francisco Office").name, "San Francisco");
        assert_eq!(
            one("TX").area,
            Some(Area::Admin1 {
                country: "US".into(),
                admin1: "TX".into()
            })
        );
    }

    #[test]
    fn unknown_place_keeps_its_region_and_is_flagged() {
        let l = one("Springfield Gardens Campus, TX");
        assert_eq!(
            l.unresolved.as_deref(),
            Some("Springfield Gardens Campus, TX")
        );
        assert_eq!(
            l.area,
            Some(Area::Admin1 {
                country: "US".into(),
                admin1: "TX".into()
            })
        );
        assert!(l.place.is_none());
    }

    #[test]
    fn unknown_text_is_unresolved() {
        let l = one("Building 7 Campus");
        assert_eq!(l.unresolved.as_deref(), Some("Building 7 Campus"));
    }

    #[test]
    fn home_by_zip_or_name() {
        let gaz = gazetteer();
        let zip = gaz.locate("98101").unwrap();
        assert_eq!(
            (zip.name.as_str(), zip.admin1.as_deref()),
            ("Seattle", Some("WA"))
        );
        assert_eq!(gaz.locate("Seattle, WA 98101").unwrap().name, "Seattle");
        assert_eq!(
            gaz.locate("Austin, TX").unwrap().admin1.as_deref(),
            Some("TX")
        );
        assert!(gaz.locate("Nowhere Special").is_none());
    }
}

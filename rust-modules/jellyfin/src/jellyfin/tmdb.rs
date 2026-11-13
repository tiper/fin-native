//! A person's whole career, from The Movie Database.
//!
//! A Jellyfin server knows a person's work only as far as its libraries hold it, and its API has
//! no route to its own metadata providers. What it does keep is the person's TMDB id, so the
//! Filmography page asks TMDB for the rest: one request per person, answered here as the
//! departments the page draws ([`CreditGroup`]). A title the library holds is named by that
//! item's own identity, so the page's availability join finds it; the others are named by an id
//! no library item has.
//!
//! The key is the build's (`TMDB_API_KEY` in the build's environment). A build without one asks
//! TMDB nothing and the page has no Filmography, as before.

use plx_plex::plex::discover::{Credit, CreditGroup, CreditItem};
use serde::Deserialize;
use std::collections::HashMap;

/// The build's TMDB key; `None` when the build was given none.
pub const API_KEY: Option<&str> = option_env!("TMDB_API_KEY");

const HOST: &str = "https://api.themoviedb.org";

/// A person's film and television credits, titles in `language` where TMDB has them.
pub fn combined_credits_url(tmdb_person_id: &str, key: &str, language: &str) -> String {
    use plx_plex::plex::urlenc_str as enc;
    // A bare `pt` is Brazilian on TMDB; the app's Portuguese is European.
    let language = if language == "pt" { "pt-PT" } else { language };
    format!("{HOST}/3/person/{}/combined_credits?api_key={}&language={}", enc(tmdb_person_id), enc(key), enc(language))
}

/// `GET /3/person/{id}/combined_credits`.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CombinedCredits {
    pub cast: Vec<Entry>,
    pub crew: Vec<Entry>,
}

/// One credit: a film (`title`, `release_date`) or a show (`name`, `first_air_date`), with the
/// character played (cast) or the department and job (crew).
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Entry {
    pub id: i64,
    pub media_type: String,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub character: Option<String>,
    pub department: Option<String>,
    pub job: Option<String>,
}

impl Entry {
    fn is_show(&self) -> bool {
        self.media_type == "tv"
    }
    fn year(&self) -> i64 {
        let date = if self.is_show() { &self.first_air_date } else { &self.release_date };
        date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()).unwrap_or(0)
    }
}

/// A library item that is one of the person's titles: what TMDB calls it, and what the page
/// needs to open it and draw it.
pub struct Held {
    pub is_show: bool,
    pub tmdb_id: String,
    /// The item's identity in the model (`Metadata::guid`), which the availability join reads.
    pub guid: String,
    pub thumb: String,
}

/// The departments of a career, larger before smaller: the cast credits as `Acting`, the crew
/// credits under TMDB's own department names. A title credited twice in one department is one
/// row naming both parts.
pub fn groups(credits: CombinedCredits, held: &[Held]) -> Vec<CreditGroup> {
    let held: HashMap<(bool, &str), &Held> = held.iter().map(|h| ((h.is_show, h.tmdb_id.as_str()), h)).collect();
    let mut out: Vec<CreditGroup> = Vec::new();
    let cast = credits.cast.into_iter().map(|mut e| (Some("Acting".to_string()), e.character.take(), e));
    let crew = credits.crew.into_iter().map(|mut e| (e.department.take(), e.job.take(), e));
    for (department, role, e) in cast.chain(crew) {
        let title = if e.is_show() { &e.name } else { &e.title };
        let Some(title) = title.clone().filter(|t| !t.is_empty() && (e.is_show() || e.media_type == "movie")) else { continue };
        let id = e.id.to_string();
        let (rating_key, thumb) = match held.get(&(e.is_show(), id.as_str())) {
            Some(h) => (h.guid.clone(), h.thumb.clone()),
            None => (format!("tmdb-{}-{id}", e.media_type), String::new()),
        };
        let department = department.filter(|d| !d.is_empty()).unwrap_or_else(|| "Crew".to_string());
        let role = role.unwrap_or_default();
        let group = match out.iter().position(|g| g.title == department) {
            Some(i) => &mut out[i],
            None => {
                out.push(CreditGroup { kind: department.to_ascii_lowercase(), title: department, ..Default::default() });
                out.last_mut().expect("just pushed")
            }
        };
        match group.credits.iter_mut().find(|c| c.item.as_ref().is_some_and(|i| i.rating_key == rating_key)) {
            Some(c) if role.is_empty() || c.role.split(", ").any(|r| r == role) => {}
            Some(c) if c.role.is_empty() => c.role = role,
            Some(c) => c.role = format!("{}, {role}", c.role),
            None => group.credits.push(Credit {
                order: group.credits.len() as i64,
                role,
                item: Some(CreditItem {
                    kind: if e.is_show() { "show" } else { "movie" }.into(),
                    title,
                    year: e.year(),
                    rating_key,
                    thumb,
                    ..Default::default()
                }),
            }),
        }
    }
    for g in &mut out {
        g.size = g.credits.len() as i64;
    }
    // Stable: the cast stays ahead of a crew department of the same size.
    out.sort_by_key(|g| std::cmp::Reverse(g.size));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDITS: &str = r#"{"cast":[
        {"id":10,"media_type":"movie","title":"Held Film","release_date":"2019-05-01","character":"Ana"},
        {"id":10,"media_type":"tv","name":"Same Number Show","first_air_date":"2021-01-01","character":"Self"},
        {"id":11,"media_type":"movie","title":"Twice","release_date":"","character":"Twin A"},
        {"id":11,"media_type":"movie","title":"Twice","release_date":"","character":"Twin B"},
        {"id":12,"media_type":"movie","title":"","character":"Nobody"}],
      "crew":[
        {"id":10,"media_type":"movie","title":"Held Film","release_date":"2019-05-01","department":"Directing","job":"Director"},
        {"id":13,"media_type":"movie","title":"Sound Work","department":"Sound","job":"Composer"}]}"#;

    #[test]
    fn a_career_is_grouped_by_department_and_held_titles_carry_the_library_identity() {
        let credits: CombinedCredits = serde_json::from_str(CREDITS).unwrap();
        let held = [Held { is_show: false, tmdb_id: "10".into(), guid: "imdb://tt1".into(), thumb: "/Items/a/Images/Primary".into() }];
        let g = groups(credits, &held);
        assert_eq!(g.iter().map(|g| (g.kind.as_str(), g.size)).collect::<Vec<_>>(), [("acting", 3), ("directing", 1), ("sound", 1)]);
        let row = |g: &CreditGroup, i: usize| {
            let c = &g.credits[i];
            let it = c.item.as_ref().unwrap();
            (it.title.clone(), it.kind.clone(), it.year, it.rating_key.clone(), it.thumb.clone(), c.role.clone())
        };
        assert_eq!(row(&g[0], 0), ("Held Film".into(), "movie".into(), 2019, "imdb://tt1".into(), "/Items/a/Images/Primary".into(), "Ana".into()));
        // A show with the film's TMDB number is another title, and not the held one.
        assert_eq!(row(&g[0], 1), ("Same Number Show".into(), "show".into(), 2021, "tmdb-tv-10".into(), String::new(), "Self".into()));
        assert_eq!(row(&g[0], 2), ("Twice".into(), "movie".into(), 0, "tmdb-movie-11".into(), String::new(), "Twin A, Twin B".into()));
        assert_eq!(row(&g[1], 0).3, "imdb://tt1");
        assert_eq!(row(&g[2], 0).5, "Composer");
    }

    #[test]
    fn the_request_names_european_portuguese() {
        assert_eq!(combined_credits_url("31", "k y", "pt"), "https://api.themoviedb.org/3/person/31/combined_credits?api_key=k%20y&language=pt-PT");
        assert!(combined_credits_url("31", "k", "de").ends_with("&language=de"));
    }
}

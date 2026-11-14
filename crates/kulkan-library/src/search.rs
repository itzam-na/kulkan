use gstreamer::glib::{self, NormalizeMode, Unichar};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

use crate::{Catalog, LibraryEntry};

/// A text criterion that either matches a substring or explicitly selects missing metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TextFilter {
    Contains(String),
    IsMissing,
}

/// An exact or inclusive numeric criterion, or an explicit missing-value criterion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NumericFilter<T> {
    Exact(T),
    RangeInclusive { min: T, max: T },
    IsMissing,
}

/// v1 Library filters. All configured criteria combine with AND.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SearchCriteria {
    pub text: Option<String>,
    pub not_yet_played: Option<bool>,
    pub artist: Option<TextFilter>,
    pub album: Option<TextFilter>,
    pub genre: Option<TextFilter>,
    pub release_year: Option<NumericFilter<u16>>,
    pub duration_seconds: Option<NumericFilter<u64>>,
    pub file_location: Option<String>,
    pub play_count: Option<NumericFilter<u64>>,
}

impl SearchCriteria {
    /// Rejects filters that the v1 interface cannot represent meaningfully.
    pub fn validate(&self) -> Result<(), String> {
        if matches!(
            self.duration_seconds.as_ref(),
            Some(NumericFilter::IsMissing)
        ) {
            return Err("duration cannot be filtered as missing".to_owned());
        }
        if self
            .release_year
            .as_ref()
            .is_some_and(|filter| !valid_numeric_filter(filter))
            || self
                .duration_seconds
                .as_ref()
                .is_some_and(|filter| !valid_numeric_filter(filter))
            || self
                .play_count
                .as_ref()
                .is_some_and(|filter| !valid_numeric_filter(filter))
        {
            return Err("numeric range lower bound must not exceed its upper bound".to_owned());
        }
        Ok(())
    }
}

fn valid_numeric_filter<T: Ord>(filter: &NumericFilter<T>) -> bool {
    match filter {
        NumericFilter::RangeInclusive { min, max } => min <= max,
        NumericFilter::Exact(_) | NumericFilter::IsMissing => true,
    }
}

/// Filters a catalog and returns matches in the specified Library order.
pub fn search_catalog(
    catalog: &Catalog,
    criteria: &SearchCriteria,
) -> Result<Vec<LibraryEntry>, String> {
    criteria.validate()?;
    let mut results = catalog
        .entries
        .iter()
        .filter(|entry| matches_criteria(entry, criteria))
        .cloned()
        .collect::<Vec<_>>();
    results.sort_by(compare_entries);
    Ok(results)
}

fn matches_criteria(entry: &LibraryEntry, criteria: &SearchCriteria) -> bool {
    criteria.text.as_ref().is_none_or(|query| {
        let query = search_key(query);
        query.is_empty()
            || entry
                .metadata
                .title
                .iter()
                .map(|value| value.value.as_str())
                .chain(
                    entry
                        .metadata
                        .artists
                        .iter()
                        .map(|value| value.value.as_str()),
                )
                .chain(
                    entry
                        .metadata
                        .release_title
                        .iter()
                        .map(|value| value.value.as_str()),
                )
                .chain(
                    entry
                        .metadata
                        .genres
                        .iter()
                        .map(|value| value.value.as_str()),
                )
                .chain(std::iter::once(entry.path.to_string_lossy().as_ref()))
                .any(|value| search_key(value).contains(&query))
    }) && criteria
        .not_yet_played
        .is_none_or(|wanted| (entry.play_count == 0) == wanted)
        && criteria.artist.as_ref().is_none_or(|filter| {
            match_text_filter(
                filter,
                entry
                    .metadata
                    .artists
                    .iter()
                    .map(|value| value.value.as_str()),
            )
        })
        && criteria.album.as_ref().is_none_or(|filter| {
            match_text_filter(
                filter,
                entry
                    .metadata
                    .release_title
                    .iter()
                    .map(|value| value.value.as_str()),
            )
        })
        && criteria.genre.as_ref().is_none_or(|filter| {
            match_text_filter(
                filter,
                entry
                    .metadata
                    .genres
                    .iter()
                    .map(|value| value.value.as_str()),
            )
        })
        && criteria
            .release_year
            .as_ref()
            .is_none_or(|filter| match_numeric_filter(filter, entry.metadata.release_year))
        && criteria.duration_seconds.as_ref().is_none_or(|filter| {
            match_numeric_filter(filter, entry.duration.map(|duration| duration.as_secs()))
        })
        && criteria.file_location.as_ref().is_none_or(|query| {
            let query = search_key(query);
            query.is_empty() || search_key(&entry.path.to_string_lossy()).contains(&query)
        })
        && criteria
            .play_count
            .as_ref()
            .is_none_or(|filter| match_numeric_filter(filter, Some(entry.play_count)))
}

fn match_text_filter<'a>(
    filter: &TextFilter,
    values: impl Iterator<Item = &'a str> + Clone,
) -> bool {
    match filter {
        TextFilter::Contains(query) => {
            let query = search_key(query);
            query.is_empty()
                || values
                    .clone()
                    .any(|value| search_key(value).contains(&query))
        }
        TextFilter::IsMissing => !values.clone().any(|value| !value.trim().is_empty()),
    }
}

fn match_numeric_filter<T: Ord>(filter: &NumericFilter<T>, value: Option<T>) -> bool {
    match filter {
        NumericFilter::Exact(expected) => value.as_ref() == Some(expected),
        NumericFilter::RangeInclusive { min, max } => {
            value.is_some_and(|value| value >= *min && value <= *max)
        }
        NumericFilter::IsMissing => value.is_none(),
    }
}

fn compare_entries(left: &LibraryEntry, right: &LibraryEntry) -> Ordering {
    compare_optional(
        left.metadata
            .artists
            .first()
            .map(|artist| search_key(&artist.value)),
        right
            .metadata
            .artists
            .first()
            .map(|artist| search_key(&artist.value)),
    )
    .then_with(|| compare_optional(release_sort_key(left), release_sort_key(right)))
    .then_with(|| compare_optional(left.metadata.track_number, right.metadata.track_number))
    .then_with(|| {
        compare_optional(
            left.metadata
                .title
                .as_ref()
                .map(|title| search_key(&title.value)),
            right
                .metadata
                .title
                .as_ref()
                .map(|title| search_key(&title.value)),
        )
    })
    .then_with(|| left.path.cmp(&right.path))
}

fn release_sort_key(entry: &LibraryEntry) -> Option<String> {
    if let Some(date) = entry.metadata.release_date.as_ref() {
        let value = date.value.trim();
        let year = value.get(..4).and_then(|year| year.parse::<u16>().ok());
        if year.is_some() && year == entry.metadata.release_year {
            return Some(value.to_owned());
        }
    }
    entry.metadata.release_year.map(|year| format!("{year:04}"))
}

fn compare_optional<T: Ord>(left: Option<T>, right: Option<T>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn search_key(value: &str) -> String {
    let folded = glib::casefold(value);
    glib::normalize(folded, NormalizeMode::Default)
        .chars()
        .filter(|character| !character.is_mark())
        .collect()
}

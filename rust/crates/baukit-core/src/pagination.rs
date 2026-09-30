//! Keyset pagination with opaque cursors bound to the request filters.
//!
//! A [`Cursor`] carries a keyset position (a sort value plus a tie-breaking
//! key, a [`Uuid`] by default) and a short hash of the normalized filters the
//! page was produced from. [`Cursor::decode`] recomputes that hash and rejects any cursor whose
//! bytes were edited, whose version is unknown, or that is replayed against
//! different filters. The cursor is not a secret and is not authenticated: it
//! prevents accidental misuse and inconsistent result sets, not a determined
//! forger who can also send an equivalent plain query.
//!
//! The module has no HTTP framework dependency, so domain and service crates
//! can build pages and decode cursors directly. Enable the `pagination` feature
//! to use it.
//!
//! ```rust
//! use baukit_core::pagination::{Page, PageKey, PageParams, PaginationError};
//! use serde::Serialize;
//! use uuid::Uuid;
//!
//! #[derive(Serialize)]
//! struct Filters<'a> {
//!     category: Option<&'a str>,
//! }
//!
//! fn first_page(rows: Vec<(String, Uuid)>) -> Result<Page<(String, Uuid)>, PaginationError> {
//!     let params = PageParams::new(Some(2), None)?;
//!     let filters = Filters { category: Some("books") };
//!     Page::from_rows(rows, &params, &filters, |(name, id)| PageKey::new(name.clone(), *id))
//! }
//!
//! let rows = vec![
//!     ("a".to_owned(), Uuid::nil()),
//!     ("b".to_owned(), Uuid::nil()),
//!     ("c".to_owned(), Uuid::nil()),
//! ];
//! let page = first_page(rows)?;
//! assert_eq!(page.items.len(), 2);
//! assert!(page.next_cursor.is_some());
//! # Ok::<(), PaginationError>(())
//! ```

use std::fmt::Display;
use std::str::FromStr;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::digest;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The page size used when a request omits `limit`.
pub const DEFAULT_PAGE_LIMIT: i64 = 50;

/// The largest page size a request may ask for.
pub const MAX_PAGE_LIMIT: i64 = 200;

/// The longest encoded cursor [`Cursor::decode`] accepts, in bytes.
///
/// Longer input is rejected before base64 decoding or JSON parsing, so an
/// oversized query parameter costs no allocation. [`Cursor::encode`] refuses to
/// issue a cursor above the same bound.
pub const MAX_CURSOR_BYTES: usize = 4096;

const CURSOR_VERSION: u8 = 1;
const FILTER_HASH_BYTES: usize = 8;

/// Validated `limit` and `cursor` query parameters.
///
/// Build this from the raw query with [`PageParams::new`] so the limit bounds
/// are enforced at the HTTP boundary rather than in the repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageParams {
    /// The requested page size, between 1 and [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    /// The opaque cursor sent by the client, still encoded.
    pub cursor: Option<String>,
}

impl Default for PageParams {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            cursor: None,
        }
    }
}

impl PageParams {
    /// Validates raw query parameters.
    ///
    /// A missing limit falls back to [`DEFAULT_PAGE_LIMIT`].
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidLimit`] when the limit is outside
    /// `1..=`[`MAX_PAGE_LIMIT`].
    pub fn new(limit: Option<i64>, cursor: Option<String>) -> Result<Self, PaginationError> {
        let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
        if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
            return Err(PaginationError::InvalidLimit);
        }
        Ok(Self { limit, cursor })
    }

    /// Returns the limit as a `usize` for slicing and truncation.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidLimit`] on platforms where the limit
    /// does not fit a `usize`.
    pub fn limit_usize(&self) -> Result<usize, PaginationError> {
        usize::try_from(self.limit).map_err(|_| PaginationError::InvalidLimit)
    }

    /// Returns the limit plus one, the row count to fetch to detect a next page.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidLimit`] if the increment overflows.
    pub fn fetch_limit(&self) -> Result<i64, PaginationError> {
        self.limit
            .checked_add(1)
            .ok_or(PaginationError::InvalidLimit)
    }

    /// Decodes the cursor against the normalized filters of this request.
    ///
    /// Returns `Ok(None)` for a first page.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidCursor`] when the cursor is malformed,
    /// carries an unsupported version, or was issued for other filters.
    pub fn decode_cursor<F>(
        &self,
        normalized_filters: &F,
    ) -> Result<Option<Cursor>, PaginationError>
    where
        F: Serialize + ?Sized,
    {
        self.cursor
            .as_deref()
            .map(|encoded| Cursor::decode(encoded, normalized_filters))
            .transpose()
    }
}

/// One keyset position: the sort value plus the key that breaks ties.
///
/// `T` is the type of the ordered column, for example a timestamp or a name.
/// `K` is the tie-breaker, the row's [`Uuid`] by default. A composite key
/// such as `(source, target)` uses the first column as `value` and the
/// second as `id`. A list ordered only by its ID uses the ID for both.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageKey<T, K = Uuid> {
    /// The value of the ordered column for the last row of the page.
    pub value: T,
    /// The tie-breaking key of that row, unique within the ordered value.
    pub id: K,
}

impl<T, K> PageKey<T, K> {
    /// Creates a keyset position.
    pub const fn new(value: T, id: K) -> Self {
        Self { value, id }
    }
}

/// One page of items plus the cursor that fetches the following page.
///
/// A `next_cursor` of `None` means the caller has reached the end. The wire name is `nextCursor`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    /// The rows of this page, already truncated to the requested limit.
    pub items: Vec<T>,
    /// The encoded cursor for the next page, or `None` at the end.
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// Creates a page from already truncated items.
    pub const fn new(items: Vec<T>, next_cursor: Option<String>) -> Self {
        Self { items, next_cursor }
    }

    /// Creates an empty final page.
    pub const fn empty() -> Self {
        Self {
            items: Vec::new(),
            next_cursor: None,
        }
    }

    /// Truncates an over-fetched row set and issues the next cursor.
    ///
    /// Fetch [`PageParams::fetch_limit`] rows, then hand them here. When more
    /// rows than the limit came back, the extra row is dropped and `key_of` is
    /// called on the last kept row to build the next cursor. The tie-breaker
    /// is stored through its `Display` form.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidLimit`] if the limit does not fit a
    /// `usize`, or [`PaginationError::InvalidCursor`] if the filters cannot be
    /// serialized.
    pub fn from_rows<F, K>(
        mut rows: Vec<T>,
        params: &PageParams,
        normalized_filters: &F,
        key_of: impl FnOnce(&T) -> PageKey<String, K>,
    ) -> Result<Self, PaginationError>
    where
        F: Serialize + ?Sized,
        K: Display,
    {
        let limit = params.limit_usize()?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let next_cursor = if has_more {
            rows.last()
                .map(|row| Cursor::from_page_key(&key_of(row), normalized_filters)?.encode())
                .transpose()?
        } else {
            None
        };
        Ok(Self::new(rows, next_cursor))
    }

    /// Maps every item while keeping the cursor, for domain to DTO conversion.
    pub fn map<U>(self, map: impl FnMut(T) -> U) -> Page<U> {
        Page {
            items: self.items.into_iter().map(map).collect(),
            next_cursor: self.next_cursor,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct CursorPayload {
    v: u8,
    k: Vec<String>,
    f: String,
}

/// An opaque keyset cursor bound to a version and to the request filters.
///
/// Encode with [`Cursor::encode`] and read the position back with
/// [`Cursor::page_key`], or [`Cursor::page_key_as`] for another tie-breaker. The wire form is base64url without padding and must be
/// treated as opaque by clients.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cursor(CursorPayload);

impl Cursor {
    /// Builds a cursor for a keyset position and the filters it belongs to.
    ///
    /// `normalized_filters` must serialize deterministically. Use a struct with
    /// a fixed field order or a `BTreeMap`, and normalize values (case, default
    /// ranges) before hashing so an equivalent request keeps its cursor valid.
    /// The tie-breaker is stored through its `Display` form.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidCursor`] if the filters cannot be
    /// serialized to JSON.
    pub fn from_page_key<F, K>(
        key: &PageKey<String, K>,
        normalized_filters: &F,
    ) -> Result<Self, PaginationError>
    where
        F: Serialize + ?Sized,
        K: Display,
    {
        Ok(Self(CursorPayload {
            v: CURSOR_VERSION,
            k: vec![key.value.clone(), key.id.to_string()],
            f: filter_hash(normalized_filters)?,
        }))
    }

    /// Decodes a client cursor and checks it against the current filters.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidCursor`] when the input is longer than
    /// [`MAX_CURSOR_BYTES`], is not base64url, is not the expected payload,
    /// carries a version this build does not understand, or was issued for a
    /// different filter set.
    pub fn decode<F>(encoded: &str, normalized_filters: &F) -> Result<Self, PaginationError>
    where
        F: Serialize + ?Sized,
    {
        let bytes = URL_SAFE_NO_PAD
            .decode(within_cursor_bound(encoded)?)
            .map_err(|_| PaginationError::InvalidCursor)?;
        let payload: CursorPayload =
            serde_json::from_slice(&bytes).map_err(|_| PaginationError::InvalidCursor)?;
        if payload.v != CURSOR_VERSION
            || payload.k.len() != 2
            || payload.f != filter_hash(normalized_filters)?
        {
            return Err(PaginationError::InvalidCursor);
        }
        Ok(Self(payload))
    }

    /// Renders the cursor as the base64url string sent to clients.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidCursor`] if the payload cannot be
    /// serialized or the encoded cursor would exceed [`MAX_CURSOR_BYTES`].
    pub fn encode(&self) -> Result<String, PaginationError> {
        let bytes = serde_json::to_vec(&self.0).map_err(|_| PaginationError::InvalidCursor)?;
        let encoded = URL_SAFE_NO_PAD.encode(bytes);
        within_cursor_bound(&encoded)?;
        Ok(encoded)
    }

    /// Parses the keyset position back into the ordered column type.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidCursor`] when the stored value does
    /// not parse as `T` or the tie-breaker is not a UUID.
    pub fn page_key<T>(&self) -> Result<PageKey<T>, PaginationError>
    where
        T: FromStr,
    {
        self.page_key_as::<T, Uuid>()
    }

    /// Parses the keyset position back into the ordered column type and the
    /// tie-breaker type.
    ///
    /// Use this when the cursor was issued from a [`PageKey`] whose `id` is
    /// not a [`Uuid`], for example the second column of a composite key.
    ///
    /// # Errors
    ///
    /// Returns [`PaginationError::InvalidCursor`] when the stored value does
    /// not parse as `T` or the tie-breaker does not parse as `K`.
    pub fn page_key_as<T, K>(&self) -> Result<PageKey<T, K>, PaginationError>
    where
        T: FromStr,
        K: FromStr,
    {
        let (value, id) = match self.0.k.as_slice() {
            [value, id] => (value, id),
            _ => return Err(PaginationError::InvalidCursor),
        };
        let value = value
            .parse::<T>()
            .map_err(|_| PaginationError::InvalidCursor)?;
        let id = id
            .parse::<K>()
            .map_err(|_| PaginationError::InvalidCursor)?;
        Ok(PageKey { value, id })
    }
}

fn within_cursor_bound(encoded: &str) -> Result<&str, PaginationError> {
    if encoded.len() > MAX_CURSOR_BYTES {
        return Err(PaginationError::InvalidCursor);
    }
    Ok(encoded)
}

fn filter_hash<F>(normalized_filters: &F) -> Result<String, PaginationError>
where
    F: Serialize + ?Sized,
{
    let bytes =
        serde_json::to_vec(normalized_filters).map_err(|_| PaginationError::InvalidCursor)?;
    let hash = digest::digest(&digest::SHA256, &bytes);
    Ok(hash.as_ref()[..FILTER_HASH_BYTES].iter().fold(
        String::with_capacity(FILTER_HASH_BYTES * 2),
        |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        },
    ))
}

/// Why a paginated request could not be served.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PaginationError {
    /// The requested page size is outside `1..=`[`MAX_PAGE_LIMIT`].
    #[error("page limit must be between 1 and 200")]
    InvalidLimit,
    /// The cursor is malformed, unsupported, or does not match the filters.
    #[error("cursor is malformed, unsupported, or does not match the request filters")]
    InvalidCursor,
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;

    #[derive(Serialize)]
    struct Filters<'a> {
        from: Option<&'a str>,
        to: Option<&'a str>,
    }

    fn no_filters() -> Filters<'static> {
        Filters {
            from: None,
            to: None,
        }
    }

    fn id(last: u8) -> Uuid {
        Uuid::from_bytes([
            0x01, 0x98, 0xaa, 0xbb, 0xcc, 0xdd, 0x7e, 0xef, 0x80, 0, 0, 0, 0, 0, 0, last,
        ])
    }

    fn encode<K: Display>(key: &PageKey<String, K>, filters: &Filters<'_>) -> String {
        Cursor::from_page_key(key, filters)
            .and_then(|cursor| cursor.encode())
            .expect("cursor should encode")
    }

    #[test]
    fn cursor_round_trips_the_keyset_position() {
        let filters = Filters {
            from: Some("2026-07-01T00:00:00Z"),
            to: None,
        };
        let key = PageKey::new("2026-07-30T18:22:11Z".to_owned(), id(1));
        let decoded = Cursor::decode(&encode(&key, &filters), &filters)
            .and_then(|cursor| cursor.page_key::<String>())
            .expect("cursor should decode");
        assert_eq!(decoded, key);
    }

    #[test]
    fn tampered_cursor_is_rejected() {
        let filters = no_filters();
        let key = PageKey::new("Bench press".to_owned(), id(2));
        let bytes = URL_SAFE_NO_PAD
            .decode(encode(&key, &filters))
            .expect("cursor should be base64url");
        let mut payload: CursorPayload =
            serde_json::from_slice(&bytes).expect("cursor should be JSON");
        payload.f = "0000000000000000".to_owned();
        let tampered =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload should serialize"));
        assert_eq!(
            Cursor::decode(&tampered, &filters),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn cursor_replayed_with_different_filters_is_rejected() {
        let original = Filters {
            from: Some("2026-07-01T00:00:00Z"),
            to: None,
        };
        let altered = Filters {
            from: Some("2026-07-02T00:00:00Z"),
            to: None,
        };
        let key = PageKey::new("Bench press".to_owned(), id(3));
        assert_eq!(
            Cursor::decode(&encode(&key, &original), &altered),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn cursor_from_another_version_is_rejected() {
        let filters = no_filters();
        let payload = CursorPayload {
            v: CURSOR_VERSION + 1,
            k: vec!["Bench press".to_owned(), id(4).to_string()],
            f: filter_hash(&filters).expect("filters should hash"),
        };
        let encoded =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload should serialize"));
        assert_eq!(
            Cursor::decode(&encoded, &filters),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn cursor_with_a_truncated_keyset_is_rejected() {
        let filters = no_filters();
        let payload = CursorPayload {
            v: CURSOR_VERSION,
            k: vec!["Bench press".to_owned()],
            f: filter_hash(&filters).expect("filters should hash"),
        };
        let encoded =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload should serialize"));
        assert_eq!(
            Cursor::decode(&encoded, &filters),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn non_base64_and_non_payload_cursors_are_rejected() {
        let filters = no_filters();
        assert_eq!(
            Cursor::decode("not a cursor!!", &filters),
            Err(PaginationError::InvalidCursor)
        );
        let garbage = URL_SAFE_NO_PAD.encode(b"{\"nope\":true}");
        assert_eq!(
            Cursor::decode(&garbage, &filters),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn page_key_rejects_an_unparseable_value_or_id() {
        let filters = no_filters();
        let key = PageKey::new("not-a-number".to_owned(), id(5));
        let cursor = Cursor::decode(&encode(&key, &filters), &filters).expect("cursor decodes");
        assert_eq!(
            cursor.page_key::<u64>().map(|key| key.value),
            Err(PaginationError::InvalidCursor)
        );

        let payload = CursorPayload {
            v: CURSOR_VERSION,
            k: vec!["7".to_owned(), "not-a-uuid".to_owned()],
            f: filter_hash(&filters).expect("filters should hash"),
        };
        let encoded =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload should serialize"));
        let cursor = Cursor::decode(&encoded, &filters).expect("cursor decodes");
        assert_eq!(
            cursor.page_key::<u64>().map(|key| key.value),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn page_limit_defaults_and_enforces_bounds() {
        assert_eq!(
            PageParams::new(None, None)
                .expect("default should validate")
                .limit,
            DEFAULT_PAGE_LIMIT
        );
        assert!(PageParams::new(Some(1), None).is_ok());
        assert!(PageParams::new(Some(MAX_PAGE_LIMIT), None).is_ok());
        assert_eq!(
            PageParams::new(Some(0), None),
            Err(PaginationError::InvalidLimit)
        );
        assert_eq!(
            PageParams::new(Some(MAX_PAGE_LIMIT + 1), None),
            Err(PaginationError::InvalidLimit)
        );
        assert_eq!(
            PageParams::new(Some(-1), None),
            Err(PaginationError::InvalidLimit)
        );
    }

    #[test]
    fn page_params_decode_cursor_passes_through_none() {
        let filters = no_filters();
        let params = PageParams::new(Some(10), None).expect("params validate");
        assert_eq!(params.decode_cursor(&filters), Ok(None));
        assert_eq!(params.fetch_limit(), Ok(11));
        assert_eq!(params.limit_usize(), Ok(10));

        let key = PageKey::new("Bench press".to_owned(), id(6));
        let params = PageParams::new(Some(10), Some(encode(&key, &filters))).expect("params");
        let decoded = params
            .decode_cursor(&filters)
            .expect("cursor decodes")
            .expect("cursor is present");
        assert_eq!(decoded.page_key::<String>(), Ok(key));
    }

    #[test]
    fn from_rows_truncates_and_issues_a_next_cursor() {
        let filters = no_filters();
        let params = PageParams::new(Some(2), None).expect("params validate");
        let rows = vec![(1_u64, id(1)), (2, id(2)), (3, id(3))];
        let page = Page::from_rows(rows, &params, &filters, |row| {
            PageKey::new(row.0.to_string(), row.1)
        })
        .expect("page builds");
        assert_eq!(page.items, vec![(1, id(1)), (2, id(2))]);
        let next = page.next_cursor.expect("a third row means another page");
        let key = Cursor::decode(&next, &filters)
            .and_then(|cursor| cursor.page_key::<u64>())
            .expect("next cursor decodes");
        assert_eq!(key, PageKey::new(2, id(2)));
    }

    #[test]
    fn from_rows_ends_the_page_when_no_extra_row_came_back() {
        let filters = no_filters();
        let params = PageParams::new(Some(5), None).expect("params validate");
        let page = Page::from_rows(vec![(1_u64, id(1))], &params, &filters, |row| {
            PageKey::new(row.0.to_string(), row.1)
        })
        .expect("page builds");
        assert_eq!(page.next_cursor, None);
        assert_eq!(page.items.len(), 1);

        let empty = Page::from_rows(Vec::<(u64, Uuid)>::new(), &params, &filters, |row| {
            PageKey::new(row.0.to_string(), row.1)
        })
        .expect("page builds");
        assert_eq!(empty, Page::empty());
    }

    #[test]
    fn page_maps_items_and_keeps_the_cursor() {
        let page = Page::new(vec![1_u64, 2], Some("cursor".to_owned()));
        let mapped = page.map(|value| value.to_string());
        assert_eq!(mapped.items, vec!["1".to_owned(), "2".to_owned()]);
        assert_eq!(mapped.next_cursor.as_deref(), Some("cursor"));
    }

    #[test]
    fn page_serializes_with_a_null_cursor_at_the_end() {
        let json = serde_json::to_value(Page::new(vec![1_u64], None)).expect("page serializes");
        assert_eq!(json, serde_json::json!({"items": [1], "nextCursor": null}));
        let restored: Page<u64> = serde_json::from_value(json).expect("page deserializes");
        assert_eq!(restored, Page::new(vec![1], None));
    }

    #[test]
    fn a_composite_text_key_round_trips_and_stays_bound_to_the_filters() {
        let tenant_a = Filters {
            from: Some("tenant-a"),
            to: None,
        };
        let tenant_b = Filters {
            from: Some("tenant-b"),
            to: None,
        };
        let key = PageKey::new("kube-system".to_owned(), "payments".to_owned());
        let encoded = encode(&key, &tenant_a);

        let decoded = Cursor::decode(&encoded, &tenant_a)
            .and_then(|cursor| cursor.page_key_as::<String, String>())
            .expect("cursor decodes");
        assert_eq!(decoded, key);
        assert_eq!(
            Cursor::decode(&encoded, &tenant_b),
            Err(PaginationError::InvalidCursor)
        );
        assert_eq!(
            Cursor::decode(&encoded, &tenant_a).and_then(|cursor| cursor.page_key::<String>()),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn a_composite_key_of_two_long_unicode_names_fits_the_bound() {
        const NAME_CHARS: usize = 255;
        let filters = no_filters();
        let name = "\u{1f600}".repeat(NAME_CHARS);
        let key = PageKey::new(name.clone(), name);

        let decoded = Cursor::decode(&encode(&key, &filters), &filters)
            .and_then(|cursor| cursor.page_key_as::<String, String>())
            .expect("cursor decodes");
        assert_eq!(decoded, key);
    }

    #[test]
    fn from_rows_issues_a_cursor_for_a_non_uuid_tie_breaker() {
        let filters = no_filters();
        let params = PageParams::new(Some(1), None).expect("params validate");
        let rows = vec![("a", 7_u32), ("b", 8)];
        let page = Page::from_rows(rows, &params, &filters, |row| {
            PageKey::new(row.0.to_owned(), row.1)
        })
        .expect("page builds");

        let next = page.next_cursor.expect("a second row means another page");
        let key = Cursor::decode(&next, &filters)
            .and_then(|cursor| cursor.page_key_as::<String, u32>())
            .expect("next cursor decodes");
        assert_eq!(key, PageKey::new("a".to_owned(), 7));
    }

    fn cursor_with_encoded_length(filters: &Filters<'_>, length: usize) -> String {
        let empty = CursorPayload {
            v: CURSOR_VERSION,
            k: vec![String::new(), id(7).to_string()],
            f: filter_hash(filters).expect("filters should hash"),
        };
        let base_json = serde_json::to_vec(&empty)
            .expect("payload should serialize")
            .len();
        let json_len = length * 3 / 4;
        let payload = CursorPayload {
            k: vec!["x".repeat(json_len - base_json), id(7).to_string()],
            ..empty
        };
        let encoded =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("payload should serialize"));
        assert_eq!(encoded.len(), length);
        encoded
    }

    #[test]
    fn cursor_bound_accepts_the_limit_and_rejects_one_byte_more() {
        let at_limit = "A".repeat(MAX_CURSOR_BYTES);
        assert_eq!(within_cursor_bound(&at_limit), Ok(at_limit.as_str()));
        let over = "A".repeat(MAX_CURSOR_BYTES + 1);
        assert_eq!(
            within_cursor_bound(&over),
            Err(PaginationError::InvalidCursor)
        );
        let filters = no_filters();
        assert_eq!(
            Cursor::decode(&over, &filters),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn cursor_at_the_bound_decodes_and_a_longer_valid_cursor_is_rejected() {
        let filters = no_filters();
        let at_limit = cursor_with_encoded_length(&filters, MAX_CURSOR_BYTES);
        assert!(Cursor::decode(&at_limit, &filters).is_ok());

        // Unpadded base64 has no encoded length of 4n + 1.
        let smallest_over = MAX_CURSOR_BYTES + 2;
        let over = cursor_with_encoded_length(&filters, smallest_over);
        assert_eq!(
            Cursor::decode(&over, &filters),
            Err(PaginationError::InvalidCursor)
        );
    }

    #[test]
    fn encode_refuses_a_cursor_above_the_bound() {
        let filters = no_filters();
        let key = PageKey::new("x".repeat(MAX_CURSOR_BYTES), id(8));
        let cursor = Cursor::from_page_key(&key, &filters).expect("cursor builds");
        assert_eq!(cursor.encode(), Err(PaginationError::InvalidCursor));
    }
}

//! Pagination utilities for SeaORM queries.

use sea_orm::{
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Select, entity::prelude::*,
};
use serde::{Deserialize, Serialize};

/// Pagination options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationOptions {
    /// Page number (1-indexed).
    #[serde(default = "default_page")]
    pub page: u64,

    /// Items per page.
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

fn default_page() -> u64 {
    1
}

fn default_per_page() -> u64 {
    20
}

impl Default for PaginationOptions {
    fn default() -> Self {
        Self {
            page: default_page(),
            per_page: default_per_page(),
        }
    }
}

impl PaginationOptions {
    /// Create new pagination options.
    pub fn new(page: u64, per_page: u64) -> Self {
        Self { page, per_page }
    }

    /// Get the offset for SQL queries.
    pub fn offset(&self) -> u64 {
        (self.page.saturating_sub(1)) * self.per_page
    }

    /// Get the limit for SQL queries.
    pub fn limit(&self) -> u64 {
        self.per_page
    }
}

/// A paginated result set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paginated<T> {
    /// The items in this page.
    pub items: Vec<T>,

    /// Pagination metadata.
    pub meta: PaginationMeta,
}

/// Pagination metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    /// Current page number.
    pub page: u64,

    /// Items per page.
    pub per_page: u64,

    /// Total number of items.
    pub total_items: u64,

    /// Total number of pages.
    pub total_pages: u64,

    /// Whether there is a next page.
    pub has_next: bool,

    /// Whether there is a previous page.
    pub has_prev: bool,
}

impl<T> Paginated<T> {
    /// Create a new paginated result.
    pub fn new(items: Vec<T>, page: u64, per_page: u64, total_items: u64) -> Self {
        let total_pages = total_items.div_ceil(per_page);

        Self {
            items,
            meta: PaginationMeta {
                page,
                per_page,
                total_items,
                total_pages,
                has_next: page < total_pages,
                has_prev: page > 1,
            },
        }
    }

    /// Map items to a different type.
    pub fn map<U, F>(self, f: F) -> Paginated<U>
    where
        F: FnMut(T) -> U,
    {
        Paginated {
            items: self.items.into_iter().map(f).collect(),
            meta: self.meta,
        }
    }
}

/// Extension trait for paginated queries.
#[async_trait::async_trait]
pub trait Paginate<E>
where
    E: EntityTrait,
{
    /// Paginate the query.
    async fn paginate(
        self,
        db: &impl sea_orm::ConnectionTrait,
        options: &PaginationOptions,
    ) -> Result<Paginated<E::Model>, sea_orm::DbErr>
    where
        E::Model: Sync;
}

#[async_trait::async_trait]
impl<E> Paginate<E> for Select<E>
where
    E: EntityTrait,
    E::Model: Send,
{
    async fn paginate(
        self,
        db: &impl sea_orm::ConnectionTrait,
        options: &PaginationOptions,
    ) -> Result<Paginated<E::Model>, sea_orm::DbErr>
    where
        E::Model: Sync,
    {
        let paginator = PaginatorTrait::paginate(self, db, options.per_page);
        let total_items = paginator.num_items().await?;
        let items = paginator.fetch_page(options.page.saturating_sub(1)).await?;

        Ok(Paginated::new(
            items,
            options.page,
            options.per_page,
            total_items,
        ))
    }
}

/// Cursor-based pagination options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorOptions {
    /// Cursor value (typically an ID or timestamp).
    pub cursor: Option<String>,

    /// Number of items to fetch.
    #[serde(default = "default_per_page")]
    pub limit: u64,

    /// Direction (forward or backward).
    #[serde(default)]
    pub direction: CursorDirection,
}

/// Cursor direction.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorDirection {
    /// Forward pagination.
    #[default]
    Forward,
    /// Backward pagination.
    Backward,
}

impl Default for CursorOptions {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: default_per_page(),
            direction: CursorDirection::Forward,
        }
    }
}

/// Cursor-paginated result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPaginated<T> {
    /// The items.
    pub items: Vec<T>,

    /// Next cursor for forward pagination.
    pub next_cursor: Option<String>,

    /// Previous cursor for backward pagination.
    pub prev_cursor: Option<String>,

    /// Whether there are more items.
    pub has_more: bool,
}

impl<T> CursorPaginated<T> {
    /// Create a new cursor-paginated result.
    pub fn new(
        items: Vec<T>,
        next_cursor: Option<String>,
        prev_cursor: Option<String>,
        has_more: bool,
    ) -> Self {
        Self {
            items,
            next_cursor,
            prev_cursor,
            has_more,
        }
    }
}

/// Additive query extensions for count-free and keyset (cursor) pagination.
///
/// These complement [`Paginate`] without altering its behavior:
///
/// * [`PaginateExt::paginate_no_count`] skips the `SELECT COUNT(*)` that
///   [`Paginate::paginate`] issues, fetching `per_page + 1` rows to detect
///   `has_next`. Use it when callers do not need `total_items`.
/// * [`PaginateExt::keyset_query`] / [`PaginateExt::keyset_paginate`] emit
///   `WHERE col > cursor` (forward) or `WHERE col < cursor` (backward),
///   `ORDER BY col ASC/DESC`, `LIMIT n` for efficient deep scrolling that
///   avoids large `OFFSET`s.
#[async_trait::async_trait]
pub trait PaginateExt<E>
where
    E: EntityTrait,
{
    /// Build the count-free page query: `LIMIT per_page + 1 OFFSET offset`.
    ///
    /// Fetching one extra row lets a caller detect [`PaginationMeta::has_next`]
    /// without a `SELECT COUNT(*)`.
    fn no_count_query(self, options: &PaginationOptions) -> Select<E>;

    /// Paginate without issuing a `SELECT COUNT(*)`.
    ///
    /// Fetches `per_page + 1` rows to compute [`PaginationMeta::has_next`], then
    /// truncates back to `per_page`. `total_items` / `total_pages` are reported as
    /// `0` (unknown) since no count is performed. Existing [`Paginate::paginate`]
    /// semantics are unchanged; this is an additive, cheaper alternative.
    async fn paginate_no_count(
        self,
        db: &impl sea_orm::ConnectionTrait,
        options: &PaginationOptions,
    ) -> Result<Paginated<E::Model>, sea_orm::DbErr>
    where
        E::Model: Sync;

    /// Build a keyset (cursor) query.
    ///
    /// Emits `WHERE column > cursor` for [`CursorDirection::Forward`] or
    /// `WHERE column < cursor` for [`CursorDirection::Backward`] (only when a
    /// `cursor` is supplied), `ORDER BY column ASC/DESC`, and `LIMIT limit`.
    fn keyset_query<C>(
        self,
        column: C,
        cursor: Option<sea_orm::Value>,
        direction: CursorDirection,
        limit: u64,
    ) -> Select<E>
    where
        C: ColumnTrait;

    /// Execute a keyset query and return a [`CursorPaginated`] result.
    ///
    /// Fetches `limit + 1` rows to compute [`CursorPaginated::has_more`], then
    /// truncates back to `limit`. The `id_of` closure derives the opaque
    /// cursor string from a returned model.
    ///
    /// For [`CursorDirection::Forward`], `next_cursor` is derived from the last
    /// returned model when more rows remain (`has_more`).
    /// For [`CursorDirection::Backward`], results are still ordered descending
    /// (nearest the supplied cursor first), so `prev_cursor` — the cursor that
    /// continues paging further backward — is derived from the *first*
    /// returned model when more rows remain further back.
    async fn keyset_paginate<C, F>(
        self,
        db: &impl sea_orm::ConnectionTrait,
        column: C,
        cursor: Option<sea_orm::Value>,
        direction: CursorDirection,
        limit: u64,
        id_of: F,
    ) -> Result<CursorPaginated<E::Model>, sea_orm::DbErr>
    where
        C: ColumnTrait,
        F: Fn(&E::Model) -> String + Send,
        E::Model: Sync;
}

#[async_trait::async_trait]
impl<E> PaginateExt<E> for Select<E>
where
    E: EntityTrait,
    E::Model: Send,
{
    fn no_count_query(self, options: &PaginationOptions) -> Select<E> {
        self.offset(options.offset()).limit(options.per_page + 1)
    }

    async fn paginate_no_count(
        self,
        db: &impl sea_orm::ConnectionTrait,
        options: &PaginationOptions,
    ) -> Result<Paginated<E::Model>, sea_orm::DbErr>
    where
        E::Model: Sync,
    {
        let per_page = options.per_page;
        let mut items = self.no_count_query(options).all(db).await?;

        let has_next = items.len() as u64 > per_page;
        if has_next {
            items.truncate(per_page as usize);
        }

        Ok(Paginated {
            items,
            meta: PaginationMeta {
                page: options.page,
                per_page,
                total_items: 0,
                total_pages: 0,
                has_next,
                has_prev: options.page > 1,
            },
        })
    }

    fn keyset_query<C>(
        self,
        column: C,
        cursor: Option<sea_orm::Value>,
        direction: CursorDirection,
        limit: u64,
    ) -> Select<E>
    where
        C: ColumnTrait,
    {
        let mut query = self;

        if let Some(cursor) = cursor {
            query = match direction {
                CursorDirection::Forward => query.filter(column.gt(cursor)),
                CursorDirection::Backward => query.filter(column.lt(cursor)),
            };
        }

        query = match direction {
            CursorDirection::Forward => query.order_by_asc(column),
            CursorDirection::Backward => query.order_by_desc(column),
        };

        query.limit(limit)
    }

    async fn keyset_paginate<C, F>(
        self,
        db: &impl sea_orm::ConnectionTrait,
        column: C,
        cursor: Option<sea_orm::Value>,
        direction: CursorDirection,
        limit: u64,
        id_of: F,
    ) -> Result<CursorPaginated<E::Model>, sea_orm::DbErr>
    where
        C: ColumnTrait,
        F: Fn(&E::Model) -> String + Send,
        E::Model: Sync,
    {
        let items = self
            .keyset_query(column, cursor, direction, limit + 1)
            .all(db)
            .await?;

        Ok(finish_keyset_page(items, direction, limit, id_of))
    }
}

/// Pure helper that turns the raw `limit + 1` rows fetched by a keyset query
/// into a [`CursorPaginated`] result: truncates the probe row, computes
/// `has_more`, and derives `next_cursor` (forward) / `prev_cursor` (backward)
/// from the appropriate end of the page.
///
/// Factored out of [`PaginateExt::keyset_paginate`] so the cursor arithmetic
/// can be unit-tested without a database connection.
fn finish_keyset_page<M, F>(
    mut items: Vec<M>,
    direction: CursorDirection,
    limit: u64,
    id_of: F,
) -> CursorPaginated<M>
where
    F: Fn(&M) -> String,
{
    let has_more = items.len() as u64 > limit;
    if has_more {
        items.truncate(limit as usize);
    }

    let next_cursor = if matches!(direction, CursorDirection::Forward) && has_more {
        items.last().map(&id_of)
    } else {
        None
    };

    let prev_cursor = if matches!(direction, CursorDirection::Backward) && has_more {
        items.first().map(&id_of)
    } else {
        None
    };

    CursorPaginated::new(items, next_cursor, prev_cursor, has_more)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, QueryTrait};

    #[allow(missing_docs)]
    mod item {
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
        #[sea_orm(table_name = "items")]
        pub struct Model {
            #[sea_orm(primary_key)]
            pub id: i32,
            pub name: String,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    use item::{Column, Entity as Item};

    #[test]
    fn paginated_new_computes_total_pages_and_first_page_flags() {
        // 95 items at 20/page => 5 pages (div_ceil). Page 1: no prev, has next.
        let page = Paginated::new(vec!["a"], 1, 20, 95);

        assert_eq!(page.meta.total_pages, 5);
        assert!(page.meta.has_next);
        assert!(!page.meta.has_prev);
    }

    #[test]
    fn paginated_new_middle_page_has_next_and_prev() {
        let page = Paginated::<&str>::new(vec![], 3, 20, 95);

        assert_eq!(page.meta.total_pages, 5);
        assert!(page.meta.has_next);
        assert!(page.meta.has_prev);
    }

    #[test]
    fn paginated_new_last_page_has_no_next() {
        let page = Paginated::<&str>::new(vec![], 5, 20, 95);

        assert_eq!(page.meta.total_pages, 5);
        assert!(!page.meta.has_next);
        assert!(page.meta.has_prev);
    }

    #[test]
    fn paginated_new_exact_multiple_total_does_not_add_extra_page() {
        // 100 items at 20/page => exactly 5 pages, not 6.
        let page = Paginated::<&str>::new(vec![], 5, 20, 100);

        assert_eq!(page.meta.total_pages, 5);
        assert!(!page.meta.has_next);
    }

    #[test]
    fn paginated_new_zero_total_items_has_no_next_or_prev() {
        let page = Paginated::<&str>::new(vec![], 1, 20, 0);

        assert_eq!(page.meta.total_pages, 0);
        assert!(!page.meta.has_next);
        assert!(!page.meta.has_prev);
    }

    #[test]
    fn no_count_query_fetches_per_page_plus_one_without_count() {
        let opts = PaginationOptions::new(3, 20);
        let stmt = Item::find()
            .no_count_query(&opts)
            .build(DatabaseBackend::Postgres);

        let sql = stmt.to_string().to_uppercase();
        assert!(sql.contains("LIMIT"), "should apply a LIMIT: {sql}");
        assert!(sql.contains("OFFSET"), "should apply an OFFSET: {sql}");
        assert!(!sql.contains("COUNT("), "must not issue a COUNT: {sql}");

        // per_page + 1 = 21 (probe row), offset = (3 - 1) * 20 = 40.
        let values = format!("{:?}", stmt.values);
        assert!(
            values.contains("21"),
            "LIMIT should be per_page + 1: {values}"
        );
        assert!(
            values.contains("40"),
            "OFFSET should be (page-1)*per_page: {values}"
        );
    }

    #[test]
    fn keyset_query_forward_emits_gt_asc_limit() {
        let stmt = Item::find()
            .keyset_query(
                Column::Id,
                Some(sea_orm::Value::from(100)),
                CursorDirection::Forward,
                10,
            )
            .build(DatabaseBackend::Postgres);

        let sql = stmt.to_string();
        assert!(sql.contains("\"id\" >"), "forward cursor uses `>`: {sql}");
        assert!(
            sql.to_uppercase().contains("ORDER BY"),
            "has ORDER BY: {sql}"
        );
        assert!(
            sql.to_uppercase().contains("ASC"),
            "forward orders ASC: {sql}"
        );
        assert!(sql.to_uppercase().contains("LIMIT"), "has LIMIT: {sql}");

        let values = format!("{:?}", stmt.values);
        assert!(values.contains("100"), "cursor value bound: {values}");
    }

    #[test]
    fn keyset_query_backward_emits_lt_desc() {
        let stmt = Item::find()
            .keyset_query(
                Column::Id,
                Some(sea_orm::Value::from(100)),
                CursorDirection::Backward,
                5,
            )
            .build(DatabaseBackend::Postgres);

        let sql = stmt.to_string();
        assert!(sql.contains("\"id\" <"), "backward cursor uses `<`: {sql}");
        assert!(
            sql.to_uppercase().contains("DESC"),
            "backward orders DESC: {sql}"
        );
    }

    #[test]
    fn keyset_query_without_cursor_has_no_where() {
        let stmt = Item::find()
            .keyset_query(Column::Id, None, CursorDirection::Forward, 10)
            .build(DatabaseBackend::Postgres);

        let sql = stmt.to_string().to_uppercase();
        assert!(
            !sql.contains("WHERE"),
            "no cursor means no WHERE clause: {sql}"
        );
        assert!(sql.contains("ORDER BY"), "still orders by the key: {sql}");
    }

    #[test]
    fn finish_keyset_page_forward_sets_next_cursor_when_more_remain() {
        // limit = 3, the query fetches limit + 1 = 4 rows to probe for more.
        let rows = vec![1, 2, 3, 4];

        let result = finish_keyset_page(rows, CursorDirection::Forward, 3, |id| id.to_string());

        assert!(result.has_more);
        assert_eq!(result.items, vec![1, 2, 3]);
        assert_eq!(result.next_cursor, Some("3".to_string()));
        assert_eq!(
            result.prev_cursor, None,
            "forward pagination does not set prev_cursor"
        );
    }

    #[test]
    fn finish_keyset_page_forward_no_next_cursor_when_no_more_remain() {
        let rows = vec![1, 2, 3];

        let result = finish_keyset_page(rows, CursorDirection::Forward, 3, |id| id.to_string());

        assert!(!result.has_more);
        assert_eq!(result.next_cursor, None);
    }

    #[test]
    fn finish_keyset_page_backward_sets_prev_cursor_when_more_remain() {
        // Backward keyset queries order DESC, so rows arrive largest-id-first.
        // limit = 3, 4 rows fetched -> has_more, truncated to the first 3.
        let rows = vec![97, 96, 95, 94];

        let result = finish_keyset_page(rows, CursorDirection::Backward, 3, |id| id.to_string());

        assert!(result.has_more);
        assert_eq!(result.items, vec![97, 96, 95]);
        assert_eq!(
            result.prev_cursor,
            Some("97".to_string()),
            "prev_cursor should come from the first returned model"
        );
        assert_eq!(
            result.next_cursor, None,
            "backward pagination does not set next_cursor"
        );
    }

    #[test]
    fn finish_keyset_page_backward_no_prev_cursor_when_no_more_remain() {
        let rows = vec![97, 96, 95];

        let result = finish_keyset_page(rows, CursorDirection::Backward, 3, |id| id.to_string());

        assert!(!result.has_more);
        assert_eq!(result.prev_cursor, None);
    }
}

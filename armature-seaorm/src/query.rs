//! Query builder utilities for SeaORM.

use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Select};
use serde::Deserialize;

/// A boxed predicate applied to an in-progress [`Condition`] while building a query.
type ConditionFn = Box<dyn Fn(Condition) -> Condition + Send + Sync>;

/// A boxed transform (e.g. an ordering) applied to an in-progress [`Select`].
type SelectFn<E> = Box<dyn Fn(Select<E>) -> Select<E> + Send + Sync>;

/// Query builder for common query patterns.
pub struct QueryBuilder<E: EntityTrait> {
    select: Option<Select<E>>,
    conditions: Vec<ConditionFn>,
    orders: Vec<SelectFn<E>>,
    limit: Option<u64>,
    offset: Option<u64>,
}

impl<E: EntityTrait> Default for QueryBuilder<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// Ascending order.
    #[default]
    Asc,
    /// Descending order.
    Desc,
}

impl<E: EntityTrait> QueryBuilder<E> {
    /// Create a new query builder.
    pub fn new() -> Self {
        Self {
            select: Some(E::find()),
            conditions: Vec::new(),
            orders: Vec::new(),
            limit: None,
            offset: None,
        }
    }

    /// Add a where clause for equality.
    pub fn where_eq<C: ColumnTrait + Send + Sync>(
        mut self,
        column: C,
        value: impl Into<sea_orm::Value>,
    ) -> Self {
        let value = value.into();
        self.conditions
            .push(Box::new(move |c| c.add(column.eq(value.clone()))));
        self
    }

    /// Add a where clause for inequality.
    pub fn where_ne<C: ColumnTrait + Send + Sync>(
        mut self,
        column: C,
        value: impl Into<sea_orm::Value>,
    ) -> Self {
        let value = value.into();
        self.conditions
            .push(Box::new(move |c| c.add(column.ne(value.clone()))));
        self
    }

    /// Add a where clause for greater than.
    pub fn where_gt<C: ColumnTrait + Send + Sync>(
        mut self,
        column: C,
        value: impl Into<sea_orm::Value>,
    ) -> Self {
        let value = value.into();
        self.conditions
            .push(Box::new(move |c| c.add(column.gt(value.clone()))));
        self
    }

    /// Add a where clause for greater than or equal.
    pub fn where_gte<C: ColumnTrait + Send + Sync>(
        mut self,
        column: C,
        value: impl Into<sea_orm::Value>,
    ) -> Self {
        let value = value.into();
        self.conditions
            .push(Box::new(move |c| c.add(column.gte(value.clone()))));
        self
    }

    /// Add a where clause for less than.
    pub fn where_lt<C: ColumnTrait + Send + Sync>(
        mut self,
        column: C,
        value: impl Into<sea_orm::Value>,
    ) -> Self {
        let value = value.into();
        self.conditions
            .push(Box::new(move |c| c.add(column.lt(value.clone()))));
        self
    }

    /// Add a where clause for less than or equal.
    pub fn where_lte<C: ColumnTrait + Send + Sync>(
        mut self,
        column: C,
        value: impl Into<sea_orm::Value>,
    ) -> Self {
        let value = value.into();
        self.conditions
            .push(Box::new(move |c| c.add(column.lte(value.clone()))));
        self
    }

    /// Add a where clause for LIKE.
    pub fn where_like<C: ColumnTrait + Send + Sync>(mut self, column: C, pattern: &str) -> Self {
        let pattern = pattern.to_owned();
        self.conditions
            .push(Box::new(move |c| c.add(column.like(pattern.clone()))));
        self
    }

    /// Add a where clause for IS NULL.
    pub fn where_null<C: ColumnTrait + Send + Sync>(mut self, column: C) -> Self {
        self.conditions
            .push(Box::new(move |c| c.add(column.is_null())));
        self
    }

    /// Add a where clause for IS NOT NULL.
    pub fn where_not_null<C: ColumnTrait + Send + Sync>(mut self, column: C) -> Self {
        self.conditions
            .push(Box::new(move |c| c.add(column.is_not_null())));
        self
    }

    /// Add a where clause for IN.
    pub fn where_in<C, I, V>(mut self, column: C, values: I) -> Self
    where
        C: ColumnTrait + Send + Sync,
        I: IntoIterator<Item = V>,
        V: Into<sea_orm::Value>,
    {
        let values: Vec<sea_orm::Value> = values.into_iter().map(Into::into).collect();
        self.conditions
            .push(Box::new(move |c| c.add(column.is_in(values.clone()))));
        self
    }

    /// Add a where clause for BETWEEN.
    pub fn where_between<C, V>(mut self, column: C, low: V, high: V) -> Self
    where
        C: ColumnTrait + Send + Sync,
        V: Into<sea_orm::Value>,
    {
        let low = low.into();
        let high = high.into();
        self.conditions.push(Box::new(move |c| {
            c.add(column.between(low.clone(), high.clone()))
        }));
        self
    }

    /// Order results by a column, ascending.
    pub fn order_asc<C: ColumnTrait + Send + Sync>(mut self, column: C) -> Self {
        self.orders.push(Box::new(move |s| s.order_by_asc(column)));
        self
    }

    /// Order results by a column, descending.
    pub fn order_desc<C: ColumnTrait + Send + Sync>(mut self, column: C) -> Self {
        self.orders.push(Box::new(move |s| s.order_by_desc(column)));
        self
    }

    /// Add a limit.
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Add an offset.
    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Build the final Select query.
    pub fn build(mut self) -> Select<E> {
        let mut select = self.select.take().unwrap_or_else(E::find);

        // Apply conditions
        if !self.conditions.is_empty() {
            let mut condition = Condition::all();
            for f in self.conditions {
                condition = f(condition);
            }
            select = select.filter(condition);
        }

        // Apply orderings
        for f in self.orders {
            select = f(select);
        }

        // Apply limit and offset
        if let Some(limit) = self.limit {
            select = select.limit(limit);
        }
        if let Some(offset) = self.offset {
            select = select.offset(offset);
        }

        select
    }
}

/// Extension trait for query helpers.
pub trait QueryExt<E: EntityTrait>: Sized {
    /// Add a where clause for equality.
    fn where_eq<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self;

    /// Add a where clause for inequality.
    fn where_ne<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self;

    /// Add a where clause for greater than.
    fn where_gt<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self;

    /// Add a where clause for greater than or equal.
    fn where_gte<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self;

    /// Add a where clause for less than.
    fn where_lt<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self;

    /// Add a where clause for less than or equal.
    fn where_lte<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self;

    /// Add a where clause for LIKE.
    fn where_like<C: ColumnTrait>(self, column: C, pattern: &str) -> Self;

    /// Add a where clause for IS NULL.
    fn where_null<C: ColumnTrait>(self, column: C) -> Self;

    /// Add a where clause for IS NOT NULL.
    fn where_not_null<C: ColumnTrait>(self, column: C) -> Self;

    /// Add a where clause for IN.
    fn where_in<C: ColumnTrait, I: IntoIterator<Item = V>, V: Into<sea_orm::Value>>(
        self,
        column: C,
        values: I,
    ) -> Self;

    /// Add a where clause for BETWEEN.
    fn where_between<C: ColumnTrait, V: Into<sea_orm::Value>>(
        self,
        column: C,
        low: V,
        high: V,
    ) -> Self;

    /// Order by ascending.
    fn order_asc<C: ColumnTrait>(self, column: C) -> Self;

    /// Order by descending.
    fn order_desc<C: ColumnTrait>(self, column: C) -> Self;
}

impl<E: EntityTrait> QueryExt<E> for Select<E> {
    fn where_eq<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self {
        self.filter(column.eq(value))
    }

    fn where_ne<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self {
        self.filter(column.ne(value))
    }

    fn where_gt<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self {
        self.filter(column.gt(value))
    }

    fn where_gte<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self {
        self.filter(column.gte(value))
    }

    fn where_lt<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self {
        self.filter(column.lt(value))
    }

    fn where_lte<C: ColumnTrait>(self, column: C, value: impl Into<sea_orm::Value>) -> Self {
        self.filter(column.lte(value))
    }

    fn where_like<C: ColumnTrait>(self, column: C, pattern: &str) -> Self {
        self.filter(column.like(pattern))
    }

    fn where_null<C: ColumnTrait>(self, column: C) -> Self {
        self.filter(column.is_null())
    }

    fn where_not_null<C: ColumnTrait>(self, column: C) -> Self {
        self.filter(column.is_not_null())
    }

    fn where_in<C: ColumnTrait, I: IntoIterator<Item = V>, V: Into<sea_orm::Value>>(
        self,
        column: C,
        values: I,
    ) -> Self {
        self.filter(column.is_in(values))
    }

    fn where_between<C: ColumnTrait, V: Into<sea_orm::Value>>(
        self,
        column: C,
        low: V,
        high: V,
    ) -> Self {
        self.filter(column.between(low, high))
    }

    fn order_asc<C: ColumnTrait>(self, column: C) -> Self {
        self.order_by_asc(column)
    }

    fn order_desc<C: ColumnTrait>(self, column: C) -> Self {
        self.order_by_desc(column)
    }
}

/// Search filters parsed from query parameters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchFilters {
    /// Text search query.
    #[serde(default)]
    pub q: Option<String>,

    /// Sort field.
    #[serde(default)]
    pub sort: Option<String>,

    /// Sort order.
    #[serde(default)]
    pub order: SortOrder,

    /// Page number.
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

impl SearchFilters {
    /// Get pagination options.
    pub fn pagination(&self) -> crate::PaginationOptions {
        crate::PaginationOptions::new(self.page, self.per_page)
    }

    /// Resolve [`Self::sort`] against a caller-supplied allowlist of `(name, column)` pairs.
    ///
    /// Returns `None` when `sort` is unset or doesn't match any entry in `allowed`. Sort
    /// resolution stays safe-by-construction: a column is never derived from untrusted
    /// input except by matching it against an explicit allowlist supplied by the caller.
    ///
    /// ```rust,ignore
    /// use armature_seaorm::SearchFilters;
    ///
    /// let filters = SearchFilters { sort: Some("name".to_owned()), ..Default::default() };
    /// let allowed = [("id", user::Column::Id), ("name", user::Column::Name)];
    /// assert_eq!(filters.sort_column(&allowed), Some(user::Column::Name));
    /// ```
    pub fn sort_column<C: ColumnTrait>(&self, allowed: &[(&str, C)]) -> Option<C> {
        let sort = self.sort.as_deref()?;
        allowed
            .iter()
            .find(|(name, _)| *name == sort)
            .map(|(_, column)| *column)
    }
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
            pub score: i32,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    use item::{Column, Entity as Item};

    #[test]
    fn query_builder_applies_conditions_order_limit_and_offset() {
        let select = QueryBuilder::<Item>::new()
            .where_eq(Column::Name, "alice")
            .where_gt(Column::Score, 10)
            .order_desc(Column::Score)
            .limit(20)
            .offset(40)
            .build();

        let stmt = select.build(DatabaseBackend::Postgres);
        let sql = stmt.to_string().to_uppercase();

        assert!(sql.contains("WHERE"), "should apply WHERE clause: {sql}");
        assert!(sql.contains("\"NAME\" ="), "should filter on name: {sql}");
        assert!(sql.contains("\"SCORE\" >"), "should filter on score: {sql}");
        assert!(
            sql.contains("ORDER BY") && sql.contains("DESC"),
            "should order by score descending: {sql}"
        );
        assert!(sql.contains("LIMIT"), "should apply LIMIT: {sql}");
        assert!(sql.contains("OFFSET"), "should apply OFFSET: {sql}");

        let values = format!("{:?}", stmt.values);
        assert!(values.contains("alice"), "name value bound: {values}");
        assert!(values.contains("10"), "score value bound: {values}");
        assert!(values.contains("20"), "LIMIT value bound: {values}");
        assert!(values.contains("40"), "OFFSET value bound: {values}");
    }

    #[test]
    fn query_builder_with_no_conditions_still_applies_limit_and_offset() {
        let select = QueryBuilder::<Item>::new().limit(5).offset(15).build();

        let stmt = select.build(DatabaseBackend::Postgres);
        let sql = stmt.to_string().to_uppercase();

        assert!(!sql.contains("WHERE"), "no conditions were added: {sql}");
        assert!(sql.contains("LIMIT"), "should apply LIMIT: {sql}");
        assert!(sql.contains("OFFSET"), "should apply OFFSET: {sql}");
    }

    #[test]
    fn sort_column_maps_known_name_via_allowlist() {
        let filters = SearchFilters {
            sort: Some("name".to_owned()),
            ..Default::default()
        };
        let allowed = [("id", Column::Id), ("name", Column::Name)];

        assert!(matches!(filters.sort_column(&allowed), Some(Column::Name)));
    }

    #[test]
    fn sort_column_returns_none_for_unlisted_name() {
        let filters = SearchFilters {
            sort: Some("score".to_owned()),
            ..Default::default()
        };
        let allowed = [("id", Column::Id), ("name", Column::Name)];

        assert!(filters.sort_column(&allowed).is_none());
    }

    #[test]
    fn sort_column_returns_none_when_sort_unset() {
        let filters = SearchFilters::default();
        let allowed = [("id", Column::Id), ("name", Column::Name)];

        assert!(filters.sort_column(&allowed).is_none());
    }
}

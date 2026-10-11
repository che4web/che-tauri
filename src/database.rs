use che_orm::{
    Database, DatabaseValue, FromDatabaseValue, Model, OrmError, QueryAst, SqlCompiler,
    SqliteDialect,
};

pub(crate) fn primary_key<M: Model>(id: i64) -> Result<M::PrimaryKey, OrmError> {
    Ok(M::PrimaryKey::from_database_value(
        &DatabaseValue::Integer(id),
    )?)
}

pub(crate) async fn write_model<M: Model + Send + 'static>(
    db: &Database,
    ast: QueryAst,
) -> Result<Option<M>, OrmError> {
    ast.validate().map_err(OrmError::QueryBuild)?;
    let compiled = SqlCompiler::<SqliteDialect>::compile(&ast);
    db.query_sql(compiled.sql, compiled.params)
        .await?
        .into_iter()
        .next()
        .map(|row| M::from_row(&row).map_err(OrmError::from))
        .transpose()
}

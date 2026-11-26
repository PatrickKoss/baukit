use super::*;
#[cfg(feature = "sqlx-postgres")]
use sqlx::Connection as _;

#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn starts_postgres_container() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start_postgres().await?;
    assert!(
        fixture
            .connection_url()
            .starts_with("postgres://postgres:postgres@")
    );
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn starts_postgres_and_applies_migrations() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::write(
        directory.path().join("0001_fixture.sql"),
        "CREATE TABLE fixture (id BIGINT PRIMARY KEY);",
    )?;
    let fixture = start_postgres_with_migrations(directory.path()).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let version = sqlx::query("SELECT current_setting('server_version_num') AS version")
        .fetch_one(&pool)
        .await?
        .try_get::<String, _>("version")?;
    assert_eq!(version, "180007");
    let settings = sqlx::query(
        "SELECT current_setting('data_directory') AS data_directory, \
         current_setting('data_checksums') AS data_checksums, \
         current_setting('password_encryption') AS password_encryption",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        settings.try_get::<String, _>("data_directory")?,
        "/var/lib/postgresql/18/docker"
    );
    assert_eq!(settings.try_get::<String, _>("data_checksums")?, "on");
    assert_eq!(
        settings.try_get::<String, _>("password_encryption")?,
        "scram-sha-256"
    );
    let row = sqlx::query("SELECT to_regclass('fixture')::text AS table_name")
        .fetch_one(&pool)
        .await?;
    assert_eq!(row.try_get::<String, _>("table_name")?, "fixture");
    pool.close().await;
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn starts_an_overridden_image_tag() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = PostgresTestOptions::new()
        .with_image("postgres", "18.7-alpine3.23@sha256:60aac580d8dbe3093c097bb1d11f9e62999814d3a1da2c1aaf729c0b7db117f2")
        .start()
        .await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let version = sqlx::query("SELECT current_setting('server_version_num') AS version")
        .fetch_one(&pool)
        .await?
        .try_get::<String, _>("version")?;
    assert_eq!(version, "180007");
    let mut release = fixture
        .container()
        .ok_or("fixture started no container")?
        .exec(testcontainers::core::ExecCommand::new([
            "cat",
            "/etc/alpine-release",
        ]))
        .await?;
    let release = String::from_utf8(release.stdout_to_vec().await?)?;
    assert!(
        release.starts_with("3.23."),
        "override was not used: {release}"
    );
    pool.close().await;
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn prepares_an_external_server_without_a_container() -> Result<(), Box<dyn std::error::Error>>
{
    let server = start_postgres().await?;
    let directory = migrations(&[("0001_fixture.sql", "CREATE TABLE fixture (id INT);")])?;
    let fixture = PostgresTestOptions::new()
        .with_migrations(directory.path())
        .start_on(Some(server.connection_url().to_owned()))
        .await?;
    assert!(fixture.container().is_none());
    assert_eq!(fixture.connection_url(), server.connection_url());
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let table = sqlx::query("SELECT to_regclass('fixture')::text AS table_name")
        .fetch_one(&pool)
        .await?
        .try_get::<String, _>("table_name")?;
    assert_eq!(table, "fixture");
    pool.close().await;
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
const RLS_MIGRATION: &str = "\
    CREATE TABLE notes (owner TEXT NOT NULL, body TEXT NOT NULL); \
    ALTER TABLE notes ENABLE ROW LEVEL SECURITY; \
    CREATE POLICY notes_owner ON notes \
        USING (owner = current_setting('app.owner', true)) \
        WITH CHECK (owner = current_setting('app.owner', true));";

#[cfg(feature = "sqlx-postgres")]
fn rls_migrations() -> Result<tempfile::TempDir, std::io::Error> {
    let directory = tempfile::tempdir()?;
    std::fs::write(directory.path().join("0001_notes.sql"), RLS_MIGRATION)?;
    Ok(directory)
}

#[cfg(feature = "sqlx-postgres")]
async fn assert_app_role_is_bound_by_rls(
    admin_url: &str,
    app_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let admin = sqlx::PgPool::connect(admin_url).await?;
    sqlx::query("INSERT INTO notes VALUES ('alice', 'a'), ('bob', 'b')")
        .execute(&admin)
        .await?;
    let mut app = sqlx::PgConnection::connect(app_url).await?;
    let role =
        sqlx::query("SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&mut app)
            .await?;
    assert!(!role.try_get::<bool, _>("rolsuper")?);
    assert!(!role.try_get::<bool, _>("rolbypassrls")?);
    sqlx::query("SELECT set_config('app.owner', 'alice', false)")
        .execute(&mut app)
        .await?;
    sqlx::query("INSERT INTO notes VALUES ('alice', 'c')")
        .execute(&mut app)
        .await?;
    let visible: i64 = sqlx::query("SELECT count(*) AS visible FROM notes")
        .fetch_one(&mut app)
        .await?
        .try_get("visible")?;
    assert_eq!(visible, 2);
    let forged = sqlx::query("INSERT INTO notes VALUES ('bob', 'forged')")
        .execute(&mut app)
        .await;
    assert!(forged.is_err(), "RLS allowed a write for another owner");
    app.close().await?;
    admin.close().await;
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn app_role_is_subject_to_row_level_security() -> Result<(), Box<dyn std::error::Error>> {
    let migrations = rls_migrations()?;
    let fixture = PostgresTestOptions::new()
        .with_app_role(PostgresAppRole::new("app_user", "app-secret")?)
        .with_migrations(migrations.path())
        .start()
        .await?;
    let app_url = fixture.app_connection_url().ok_or("missing app URL")?;
    assert!(app_url.starts_with("postgres://app_user:app-secret@"));
    assert_app_role_is_bound_by_rls(fixture.connection_url(), app_url).await
}

#[cfg(feature = "sqlx-postgres")]
fn migrations(files: &[(&str, &str)]) -> Result<tempfile::TempDir, std::io::Error> {
    let directory = tempfile::tempdir()?;
    for (name, sql) in files {
        std::fs::write(directory.path().join(name), sql)?;
    }
    Ok(directory)
}

#[cfg(feature = "sqlx-postgres")]
async fn start_as_owner(
    owner: PostgresMigrationOwner,
    files: &[(&str, &str)],
) -> Result<PostgresTestContainer, Box<dyn std::error::Error>> {
    let directory = migrations(files)?;
    Ok(start_postgres_with_owner_migrations(directory.path(), owner).await?)
}

#[cfg(feature = "sqlx-postgres")]
const CREATE_ROLE_MIGRATION: (&str, &str) =
    ("0001_roles.sql", "CREATE ROLE fixture_reader NOLOGIN;");

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn migration_owner_owns_what_migrations_create() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start_as_owner(
        PostgresMigrationOwner::default(),
        &[(
            "0001_notes.sql",
            "CREATE TABLE notes (id BIGINT PRIMARY KEY);",
        )],
    )
    .await?;
    let owner_url = fixture.owner_connection_url().ok_or("missing owner URL")?;
    assert!(owner_url.starts_with("postgres://app:app@"));
    let mut owner = sqlx::PgConnection::connect(owner_url).await?;
    let row = sqlx::query(
        "SELECT r.rolsuper, r.rolcreaterole, r.rolcreatedb, r.rolbypassrls, \
                pg_get_userbyid(d.datdba) AS database_owner, \
                (SELECT tableowner FROM pg_tables WHERE tablename = 'notes') AS table_owner \
         FROM pg_roles r, pg_database d \
         WHERE r.rolname = current_user AND d.datname = current_database()",
    )
    .fetch_one(&mut owner)
    .await?;
    for attribute in ["rolsuper", "rolcreaterole", "rolcreatedb", "rolbypassrls"] {
        assert!(!row.try_get::<bool, _>(attribute)?, "{attribute}");
    }
    assert_eq!(row.try_get::<String, _>("database_owner")?, "app");
    assert_eq!(row.try_get::<String, _>("table_owner")?, "app");
    owner.close().await?;
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn superuser_only_migrations_fail_under_the_owner() -> Result<(), Box<dyn std::error::Error>>
{
    let statements = [
        "CREATE ROLE fixture_reader NOLOGIN;",
        "CREATE EXTENSION file_fdw;",
        "ALTER ROLE CURRENT_USER BYPASSRLS;",
    ];
    let superuser = migrations(&[("0001_su.sql", &statements.concat())])?;
    start_postgres_with_migrations(superuser.path()).await?;

    for sql in statements {
        let result =
            start_as_owner(PostgresMigrationOwner::default(), &[("0001_su.sql", sql)]).await;
        let error = result.err().ok_or(format!("owner ran {sql}"))?;
        assert!(
            matches!(
                error.downcast_ref::<PostgresTestError>(),
                Some(PostgresTestError::Migrate(_))
            ),
            "{sql}: {error}"
        );
    }
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn create_role_owner_follows_the_documented_migration_rules()
-> Result<(), Box<dyn std::error::Error>> {
    let owner = || PostgresMigrationOwner::default().with_create_role();
    start_as_owner(
        owner(),
        &[
            CREATE_ROLE_MIGRATION,
            (
                "0002_handover.sql",
                "CREATE TABLE notes (id BIGINT PRIMARY KEY); \
                 GRANT fixture_reader TO CURRENT_USER; \
                 GRANT CREATE ON SCHEMA public TO fixture_reader; \
                 ALTER TABLE notes OWNER TO fixture_reader; \
                 SET ROLE fixture_reader; \
                 RESET ROLE;",
            ),
        ],
    )
    .await?;

    let self_grant = "SET createrole_self_grant = 'set, inherit'; \
                      CREATE ROLE fixture_writer NOLOGIN; \
                      SET ROLE fixture_writer; \
                      RESET ROLE;";
    start_as_owner(owner(), &[("0001_self_grant.sql", self_grant)]).await?;

    for (name, sql) in [
        (
            "without a self-grant",
            "CREATE ROLE fixture_reader NOLOGIN; SET ROLE fixture_reader;",
        ),
        (
            "without CREATE on the schema",
            "CREATE ROLE fixture_reader NOLOGIN; \
             GRANT fixture_reader TO CURRENT_USER; \
             CREATE TABLE notes (id BIGINT PRIMARY KEY); \
             ALTER TABLE notes OWNER TO fixture_reader;",
        ),
    ] {
        let result = start_as_owner(owner(), &[("0001_rule.sql", sql)]).await;
        let error = result.err().ok_or(format!("{name} passed"))?;
        assert!(
            matches!(
                error.downcast_ref::<PostgresTestError>(),
                Some(PostgresTestError::Migrate(_))
            ),
            "{name}: {error}"
        );
    }
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn app_role_reaches_owner_tables_in_new_databases() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = rls_migrations()?;
    let fixture = PostgresTestOptions::new()
        .with_app_role(PostgresAppRole::new("app_user", "app-secret")?)
        .with_migration_owner(PostgresMigrationOwner::new(
            "product_owner",
            "owner-secret",
        )?)
        .with_migrations(directory.path())
        .start()
        .await?;
    let database = fixture.create_database().await?;
    let owner_url = database.owner_connection_url().ok_or("missing owner URL")?;
    assert!(owner_url.starts_with("postgres://product_owner:owner-secret@"));
    let mut owner = sqlx::PgConnection::connect(owner_url).await?;
    let table_owner: String =
        sqlx::query_scalar("SELECT tableowner::text FROM pg_tables WHERE tablename = 'notes'")
            .fetch_one(&mut owner)
            .await?;
    assert_eq!(table_owner, "product_owner");
    owner.close().await?;
    assert_app_role_is_bound_by_rls(
        database.connection_url(),
        database.app_connection_url().ok_or("missing app URL")?,
    )
    .await
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn rejects_an_existing_owner_with_other_attributes() -> Result<(), Box<dyn std::error::Error>>
{
    let server = start_postgres().await?;
    let databases = PostgresTestDatabases::new(server.connection_url());
    databases
        .clone()
        .with_migration_owner(PostgresMigrationOwner::default())
        .create()
        .await?;
    let result = databases
        .with_migration_owner(PostgresMigrationOwner::default().with_create_role())
        .create()
        .await;
    assert!(matches!(result, Err(PostgresTestError::InvalidAppRole(_))));
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
async fn database_exists(admin_url: &str, name: &str) -> Result<bool, sqlx::Error> {
    let mut admin = sqlx::PgConnection::connect(admin_url).await?;
    let exists = sqlx::query("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(name)
        .fetch_one(&mut admin)
        .await?
        .try_get::<bool, _>(0)?;
    admin.close().await?;
    Ok(exists)
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn creates_and_drops_a_database_per_test() -> Result<(), Box<dyn std::error::Error>> {
    let migrations = rls_migrations()?;
    let fixture = PostgresTestOptions::new()
        .with_app_role(PostgresAppRole::new("app_user", "app-secret")?)
        .with_migrations(migrations.path())
        .start()
        .await?;
    let first = fixture.create_database().await?;
    let second = fixture.create_database().await?;
    assert_ne!(first.name(), second.name());
    assert!(first.name().starts_with("baukit_test_"));
    assert_app_role_is_bound_by_rls(
        first.connection_url(),
        first.app_connection_url().ok_or("missing app URL")?,
    )
    .await?;

    let first_name = first.name().to_owned();
    let second_name = second.name().to_owned();
    let _open = sqlx::PgPool::connect(first.connection_url()).await?;
    first.drop_database().await?;
    drop(second);

    assert!(!database_exists(fixture.connection_url(), &first_name).await?);
    assert!(!database_exists(fixture.connection_url(), &second_name).await?);
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn external_server_databases_are_dropped() -> Result<(), Box<dyn std::error::Error>> {
    let server = start_postgres().await?;
    let databases = PostgresTestDatabases::new(server.connection_url());
    let database = databases.create().await?;
    let name = database.name().to_owned();
    assert!(database_exists(server.connection_url(), &name).await?);
    assert!(database.app_connection_url().is_none());

    drop(database);

    assert!(!database_exists(server.connection_url(), &name).await?);
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn rejects_an_existing_superuser_as_app_role() -> Result<(), Box<dyn std::error::Error>> {
    let server = start_postgres().await?;
    let result = PostgresTestDatabases::new(server.connection_url())
        .with_app_role(PostgresAppRole::new("postgres", "postgres")?)
        .create()
        .await;
    assert!(matches!(result, Err(PostgresTestError::InvalidAppRole(_))));
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn audits_direct_user_root_foreign_keys() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start_postgres().await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    sqlx::query("CREATE TABLE erasure_users (id BIGINT PRIMARY KEY)")
        .execute(&pool)
        .await?;
    sqlx::query(
        "CREATE TABLE cascading_records ( \
             id BIGINT PRIMARY KEY, \
             user_id BIGINT NOT NULL REFERENCES erasure_users(id) ON DELETE CASCADE \
         )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE restricted_records ( \
             id BIGINT PRIMARY KEY, \
             user_id BIGINT NOT NULL REFERENCES erasure_users(id) \
         )",
    )
    .execute(&pool)
    .await?;
    let registry = [
        OwnedResourceCheck {
            name: "cascading_records",
            count_sql: "SELECT count(*) FROM cascading_records WHERE user_id = $1",
            cleanup: CleanupKind::Cascade,
        },
        OwnedResourceCheck {
            name: "restricted_records",
            count_sql: "SELECT count(*) FROM restricted_records WHERE user_id = $1",
            cleanup: CleanupKind::Cascade,
        },
    ];

    let mismatches = audit_user_root_foreign_keys(&pool, "erasure_users", &registry).await?;

    assert_eq!(mismatches.len(), 1);
    assert_eq!(mismatches[0].referencing_table, "public.restricted_records");
    assert_eq!(mismatches[0].actual_delete_action, "NO ACTION");
    for cleanup in [CleanupKind::Explicit, CleanupKind::AsyncProcessor] {
        let non_cascade_registry = [
            registry[0],
            OwnedResourceCheck {
                cleanup,
                ..registry[1]
            },
        ];
        assert!(
            audit_user_root_foreign_keys(&pool, "erasure_users", &non_cascade_registry)
                .await?
                .is_empty()
        );
    }
    pool.close().await;
    Ok(())
}

#[cfg(feature = "sqlx-postgres")]
#[tokio::test]
#[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
async fn audits_same_named_tables_in_different_schemas() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start_postgres().await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    sqlx::raw_sql(
        "CREATE TABLE public.erasure_users (id BIGINT PRIMARY KEY); \
         CREATE SCHEMA tenant; \
         CREATE TABLE public.records (user_id BIGINT REFERENCES public.erasure_users); \
         CREATE TABLE tenant.records (user_id BIGINT REFERENCES public.erasure_users);",
    )
    .execute(&pool)
    .await?;
    for name in ["records", "public.records"] {
        let registry = [OwnedResourceCheck {
            name,
            count_sql: "SELECT count(*) FROM public.records WHERE user_id = $1",
            cleanup: CleanupKind::Explicit,
        }];
        let mismatches = audit_user_root_foreign_keys(&pool, "erasure_users", &registry).await?;
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].referencing_table, "tenant.records");
        assert_eq!(mismatches[0].actual_delete_action, "NO ACTION");
        assert_eq!(mismatches[0].declared_cleanup, CleanupKind::Cascade);
    }
    pool.close().await;
    Ok(())
}

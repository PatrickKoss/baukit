-- The app connects and migrates as `app`, the owner a CNPG cluster creates: not a superuser,
-- no CREATEDB, REPLICATION or BYPASSRLS{% if context.postgres_owner_create_role %}, and CREATEROLE from `[postgres] owner_create_role`{% else %}, no CREATEROLE{% endif %}.
CREATE ROLE app LOGIN PASSWORD 'app' NOSUPERUSER NOCREATEDB NOREPLICATION NOBYPASSRLS {% if context.postgres_owner_create_role %}CREATEROLE{% else %}NOCREATEROLE{% endif %};
DO $$ BEGIN EXECUTE format('ALTER DATABASE %I OWNER TO app', current_database()); END $$;

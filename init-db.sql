CREATE EXTENSION ltree;

CREATE TABLE route_tables (
	id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
	table_key JSONB NOT NULL,
	started_at TIMESTAMP NOT NULL,
	ended_at TIMESTAMP
);

-- space saving data scheme

CREATE TABLE route_attrs (
	id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,

	prefix CIDR NOT NULL,
	med OID,
	local_pref OID,
	nexthop INET,
	as_path LTREE,
	communities OID[],
	large_communities OID[],

	UNIQUE NULLS NOT DISTINCT (prefix, med, local_pref, nexthop, as_path, communities, large_communities)
);

CREATE INDEX idx_route_attrs_as_path ON route_attrs USING GIST (as_path);
CREATE INDEX idx_route_attrs_prefix_inet_ops ON route_attrs USING GIST (prefix inet_ops);
CREATE INDEX idx_route_attrs_prefix ON route_attrs (prefix);

CREATE TABLE routes (
	id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,

	table_id INT NOT NULL REFERENCES route_tables (id),
	path_id OID NOT NULL,
	started_at TIMESTAMP NOT NULL,
	ended_at TIMESTAMP,
	attrs_id INT NOT NULL REFERENCES route_attrs (id)
);
CREATE INDEX idx_routes_attrs_id ON routes (attrs_id);
CREATE INDEX idx_routes ON routes (table_id, path_id, started_at);

CREATE OR REPLACE VIEW view_routes AS
SELECT
	table_key,
	prefix,
	GREATEST(route_tables.started_at, routes.started_at) AS started_at,
	LEAST(route_tables.ended_at, routes.ended_at) AS ended_at,
	med,
	local_pref,
	nexthop,
	as_path,
	communities,
	large_communities
FROM routes
	JOIN route_tables ON table_id = route_tables.id
	JOIN route_attrs ON attrs_id = route_attrs.id
;

-- temp data scheme

CREATE UNLOGGED TABLE temp_routes (
	table_id INT NOT NULL REFERENCES route_tables (id),
	path_id OID NOT NULL,
	prefix CIDR NOT NULL,

	-- for update
	started_at TIMESTAMP,
	med OID,
	local_pref OID,
	nexthop INET,
	as_path LTREE,
	communities OID[],
	large_communities OID[],

	-- for withdraw
	ended_at TIMESTAMP
);

-- migration function

CREATE MATERIALIZED VIEW temp_updates AS (
	SELECT *
	FROM temp_routes
	WHERE started_at IS NOT NULL
)
WITH NO DATA;

CREATE MATERIALIZED VIEW temp_updates_joined AS (
	SELECT t.*, route_attrs.id AS attrs_id
	FROM temp_updates AS t
	LEFT JOIN route_attrs ON (
		route_attrs.prefix = t.prefix AND
		route_attrs.med IS NOT DISTINCT FROM t.med AND
		route_attrs.local_pref IS NOT DISTINCT FROM t.local_pref AND
		route_attrs.nexthop IS NOT DISTINCT FROM t.nexthop AND
		route_attrs.as_path IS NOT DISTINCT FROM t.as_path AND
		route_attrs.communities IS NOT DISTINCT FROM t.communities AND
		route_attrs.large_communities IS NOT DISTINCT FROM t.large_communities
	)
)
WITH NO DATA;

CREATE MATERIALIZED VIEW temp_withdraws AS (
	SELECT table_id, path_id, prefix, ended_at
	FROM temp_routes
	WHERE ended_at IS NOT NULL
)
WITH NO DATA;

CREATE OR REPLACE PROCEDURE add_missing_attrs() AS $$
	BEGIN
		REFRESH MATERIALIZED VIEW temp_updates;
		ANALYZE temp_updates;
		REFRESH MATERIALIZED VIEW temp_updates_joined;
		ANALYZE temp_updates_joined;

		INSERT INTO route_attrs (prefix, med, local_pref, nexthop, as_path, communities, large_communities)
			SELECT DISTINCT
				t.prefix,
				t.med,
				t.local_pref,
				t.nexthop,
				t.as_path,
				t.communities,
				t.large_communities
			FROM temp_updates_joined AS t
			WHERE attrs_id is NULL;
	END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE PROCEDURE process_updates() AS $$
	BEGIN
		REFRESH MATERIALIZED VIEW temp_updates_joined;
		ANALYZE temp_updates_joined;

		INSERT INTO routes (table_id, path_id, started_at, attrs_id)
			SELECT table_id, path_id, started_at, attrs_id
			FROM temp_updates_joined;
	END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE PROCEDURE process_withdraws() AS $$
	BEGIN
		REFRESH MATERIALIZED VIEW temp_withdraws;
		ANALYZE temp_withdraws;

		MERGE INTO routes r
		USING (
			SELECT routes.id, max(t.ended_at) AS ended_at FROM routes
			JOIN route_attrs ON route_attrs.id = routes.attrs_id
			JOIN temp_withdraws AS t ON
				t.prefix = route_attrs.prefix AND
				t.path_id = routes.path_id AND
				t.table_id = routes.table_id
			WHERE
				routes.ended_at IS NULL
			GROUP BY routes.id
		) d
		ON r.id = d.id
		WHEN MATCHED THEN UPDATE SET ended_at = d.ended_at;
	END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE PROCEDURE delete_processed() AS $$
	BEGIN
		DELETE FROM temp_routes;
	END;
$$ LANGUAGE plpgsql;
CREATE OR REPLACE PROCEDURE persist_data() AS $$
	BEGIN
	END;
$$ LANGUAGE plpgsql;

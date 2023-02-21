create table routes (
	table_key jsonb COMPRESSION lz4,
	prefix cidr,
	path_id oid,

	attrs jsonb not null,
	primary key (table_key, prefix, path_id)
);
create index on routes using gist (prefix inet_ops);

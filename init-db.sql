create table route_tables (
	id int generated always as identity primary key,
	table_key jsonb not null,
	ended_at timestamp
);

create table routes (
	id int generated always as identity primary key,

	table_id int not null,
	prefix cidr not null,
	path_id oid not null,
	started_at timestamp not null,

	ended_at timestamp,
	attrs jsonb not null,

	foreign key (table_id) references route_tables (id)
);
create unique index on routes (table_id, prefix, path_id, started_at);
create index on routes using gist (prefix inet_ops);

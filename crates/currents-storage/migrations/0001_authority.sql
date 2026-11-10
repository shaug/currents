-- C04 authority v1. A file is one dataset; dataset UUID is retained in Dataset items.
CREATE TABLE currents_commits (
 seq INTEGER PRIMARY KEY CHECK(seq>0), previous_digest BLOB NOT NULL CHECK(length(previous_digest)=32),
 digest BLOB NOT NULL UNIQUE CHECK(length(digest)=32), canonical BLOB NOT NULL CHECK(length(canonical)<=67108864),
 checkpoint TEXT NOT NULL CHECK(length(checkpoint)<=1024), command_id BLOB NOT NULL UNIQUE CHECK(length(command_id)=16)
) STRICT;
CREATE TRIGGER currents_commit_order BEFORE INSERT ON currents_commits BEGIN
 SELECT CASE WHEN NEW.seq != coalesce((SELECT max(seq)+1 FROM currents_commits),1) THEN RAISE(ABORT,'commit order') END;
 SELECT CASE WHEN NEW.previous_digest != coalesce((SELECT digest FROM currents_commits ORDER BY seq DESC LIMIT 1),zeroblob(32)) THEN RAISE(ABORT,'commit chain') END;
END;
CREATE TABLE currents_items (
 seq INTEGER NOT NULL REFERENCES currents_commits(seq), position INTEGER NOT NULL CHECK(position>=0),
 kind TEXT NOT NULL, digest BLOB NOT NULL CHECK(length(digest)=32), canonical BLOB NOT NULL CHECK(length(canonical)<=1048576),
 PRIMARY KEY(seq,position)
) STRICT;
CREATE TABLE currents_definitions (
 kind TEXT NOT NULL, id TEXT NOT NULL, version INTEGER NOT NULL CHECK(version>=0), seq INTEGER NOT NULL, position INTEGER NOT NULL,
 PRIMARY KEY(kind,id,version), FOREIGN KEY(seq,position) REFERENCES currents_items(seq,position)
) STRICT;
CREATE UNIQUE INDEX currents_one_dataset ON currents_definitions(kind) WHERE kind='Dataset';
CREATE INDEX currents_definitions_by_item ON currents_definitions(seq,position,kind,id,version);
CREATE TABLE currents_references (
 seq INTEGER NOT NULL, position INTEGER NOT NULL, target_kind TEXT NOT NULL, target_id TEXT NOT NULL, target_version INTEGER NOT NULL,
 FOREIGN KEY(seq,position) REFERENCES currents_items(seq,position),
 FOREIGN KEY(target_kind,target_id,target_version) REFERENCES currents_definitions(kind,id,version)
) STRICT;
CREATE INDEX currents_references_by_item ON currents_references(seq,position,target_kind,target_id,target_version);
CREATE TRIGGER currents_reference_prefix BEFORE INSERT ON currents_references BEGIN
 SELECT CASE WHEN (SELECT seq FROM currents_definitions WHERE kind=NEW.target_kind AND id=NEW.target_id AND version=NEW.target_version)>NEW.seq THEN RAISE(ABORT,'future reference') END;
END;
CREATE TABLE currents_blobs (
 digest BLOB PRIMARY KEY CHECK(length(digest)=32), byte_count INTEGER NOT NULL CHECK(byte_count>=0 AND byte_count<=104857600),
 first_seq INTEGER NOT NULL REFERENCES currents_commits(seq)
) STRICT;
CREATE TABLE currents_chunks (
 digest BLOB NOT NULL REFERENCES currents_blobs(digest), position INTEGER NOT NULL CHECK(position>=0),
 bytes BLOB NOT NULL CHECK(length(bytes)<=1048576), chunk_digest BLOB NOT NULL CHECK(length(chunk_digest)=32), PRIMARY KEY(digest,position)
) STRICT;
CREATE TABLE currents_content_refs (
 seq INTEGER NOT NULL, position INTEGER NOT NULL, digest BLOB NOT NULL REFERENCES currents_blobs(digest),
 FOREIGN KEY(seq,position) REFERENCES currents_items(seq,position)
) STRICT;
CREATE INDEX currents_content_refs_by_item ON currents_content_refs(seq,position,digest);
CREATE TABLE currents_staging (
 token BLOB PRIMARY KEY CHECK(length(token)=16), digest BLOB NOT NULL CHECK(length(digest)=32), byte_count INTEGER NOT NULL CHECK(byte_count>=0 AND byte_count<=104857600)
) STRICT;
CREATE TABLE currents_staging_chunks (
 token BLOB NOT NULL REFERENCES currents_staging(token), position INTEGER NOT NULL CHECK(position>=0), bytes BLOB NOT NULL CHECK(length(bytes)<=1048576),
 chunk_digest BLOB NOT NULL CHECK(length(chunk_digest)=32), PRIMARY KEY(token,position)
) STRICT;
CREATE TRIGGER currents_commits_no_update BEFORE UPDATE ON currents_commits BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_commits_no_delete BEFORE DELETE ON currents_commits BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_items_no_update BEFORE UPDATE ON currents_items BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_items_no_delete BEFORE DELETE ON currents_items BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_definitions_no_update BEFORE UPDATE ON currents_definitions BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_definitions_no_delete BEFORE DELETE ON currents_definitions BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_references_no_update BEFORE UPDATE ON currents_references BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_references_no_delete BEFORE DELETE ON currents_references BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_blobs_no_update BEFORE UPDATE ON currents_blobs BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_blobs_no_delete BEFORE DELETE ON currents_blobs BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_chunks_no_update BEFORE UPDATE ON currents_chunks BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_chunks_no_delete BEFORE DELETE ON currents_chunks BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_content_refs_no_update BEFORE UPDATE ON currents_content_refs BEGIN SELECT RAISE(ABORT,'immutable authority'); END;
CREATE TRIGGER currents_content_refs_no_delete BEFORE DELETE ON currents_content_refs BEGIN SELECT RAISE(ABORT,'immutable authority'); END;

-- Profile-only name: duplicate names are allowed and login identifiers stay unchanged.
ALTER TABLE users ADD COLUMN display_name VARCHAR(100);

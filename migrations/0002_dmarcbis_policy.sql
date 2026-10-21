-- RFC 9989 (DMARCbis) tags of the DMARC record a DNS check found, written
-- with the other policy_* columns (NULL without a record). policy_testing is
-- t=y: 1 when testing, 0 otherwise (t=n, absent or malformed). policy_np is
-- np=, the policy for subdomains that do not exist. Rows written before this
-- migration keep NULL until their re-check, which CHECKER_VERSION forces.
ALTER TABLE dns_snapshots ADD COLUMN policy_testing INTEGER;
ALTER TABLE dns_snapshots ADD COLUMN policy_np TEXT;

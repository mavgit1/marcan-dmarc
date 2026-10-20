# Vendored sender name data

Both files are copied **unmodified** from
[parsedmarc](https://github.com/domainaware/parsedmarc/tree/master/parsedmarc/resources/maps)
by `scripts/update-sender-names.sh`. Do not edit them here: name senders in
Settings → Sender names instead (see the README), or contribute
upstream.

| File | Upstream | License |
| :--- | :--- | :--- |
| `base_reverse_dns_map.csv` | `parsedmarc/resources/maps/base_reverse_dns_map.csv` | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/), by the parsedmarc contributors, with network identification data from [IPinfo](https://ipinfo.io/) |
| `psl_overrides.txt` | `parsedmarc/resources/maps/psl_overrides.txt` | [Apache 2.0](https://github.com/domainaware/parsedmarc/blob/master/LICENSE), by the parsedmarc contributors |

Marcan reads the `base_reverse_dns` and `name` columns of the map; the `type`
column is ignored.

`SOURCE` records the upstream commit the files were taken from.

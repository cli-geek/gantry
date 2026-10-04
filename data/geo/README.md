# Offline place data

Used to turn posting locations and the user's home location into
coordinates without calling any service (plan §12).

Source: [GeoNames](https://www.geonames.org/), downloaded 2026-10-03.

License: [Creative Commons Attribution 4.0](https://creativecommons.org/licenses/by/4.0/).

The files here are reduced extracts; GeoNames is not responsible for the
changes.

| File | From | Columns |
|---|---|---|
| `cities.tsv` | `cities5000.zip` (populated places, population ≥ 5,000), feature class `P` | name, ASCII name, ASCII alternate names (places ≥ 100,000 people only), latitude, longitude, country, admin-1 code, population |
| `postal_us.tsv` | `export/zip/US.zip` | country, ZIP code, place, state, latitude, longitude |
| `admin1.tsv` | `admin1CodesASCII.txt` | country, admin-1 code, ASCII name |
| `countries.tsv` | `countryInfo.txt` | ISO 3166-1 alpha-2, alpha-3, name, continent |

Places smaller than 5,000 people and postal codes outside the US are not
included. A location that does not resolve is flagged in the posting's
filter results; it is never guessed.

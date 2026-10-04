# Fixture profiles

Fictional people used by the fixture tests and for manual runs:

    gantry --config-dir fixtures/profiles/student --data-dir /tmp/g run --discover-only

| Profile | Who | Exercises |
|---|---|---|
| `student` | CS student on an F-1 visa, Seattle, internships and new-grad roles | sponsorship filter, job types, distance, feed collapse, reposts |
| `early-career` | Backend engineer, two years in, San Francisco | pay floor, remote/hybrid only, freshness, unverified synonym ignored, HN → probe → board |
| `career-changer` | Former teacher moving into data roles, remote only | remote eligibility, preferred years, manual URL |
| `nontech-guard` | Registered dental hygienist, Austin | the core with no technology pack: no seed boards, no feeds, hourly pay |

`nontech-guard` exists only to prove the core holds no technology
assumptions (plan §1, §9.1). Any test failure there means a tech default
leaked into shared code.

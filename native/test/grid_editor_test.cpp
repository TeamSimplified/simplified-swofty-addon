// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The seats layers against the SDK's fake core: it runs SWOFTY_REGISTER with test/storage as
// Simplified's folder in the game and applies the layers it records to Progressive's 2016 season and to
// Classic Seasons' 2022 grid.
// Run it from native/ with `build.sh test`.

#if defined(_WIN32)
#include <windows.h>
static const HMODULE kCoreStub = LoadLibraryW(L"swofty_core.dll");
#endif

#include "../src/grid_editor.cpp"

#include <cstdio>
#include <cstring>
#include <fstream>
#include <sstream>

#include <swofty/fake_core.hpp>

static int failures = 0;

static void check(bool ok, const char *what)
{
    std::printf("%s  %s\n", ok ? "ok  " : "FAIL", what);
    failures += !ok;
}

static std::string g_storage;

static bool slurp(const char *path, std::string &out)
{
    std::ifstream f(g_storage + path, std::ios::binary);
    if (!f)
        return false;
    std::ostringstream s;
    s << f.rdbuf();
    out = s.str();
    return true;
}

static int s_available() { return 1; }
static int s_size(const char *path, uint64_t *out)
{
    std::string text;
    if (!slurp(path, text))
        return SWOFTY_ASSET_INVALID;
    *out = text.size();
    return SWOFTY_ASSET_OK;
}
static int s_read(const char *path, void *buf, size_t cap, size_t *len)
{
    std::string text;
    if (!slurp(path, text) || text.size() > cap)
        return SWOFTY_ASSET_INVALID;
    std::memcpy(buf, text.data(), text.size());
    *len = text.size();
    return SWOFTY_ASSET_OK;
}
static int s_refuse(const char *, const void *, size_t) { return SWOFTY_ASSET_INVALID; }
static int s_remove(const char *) { return SWOFTY_ASSET_INVALID; }
static int s_list(const char *, char *, size_t, size_t *) { return SWOFTY_ASSET_INVALID; }
static const SwoftyStorage kStorage = {s_available, s_read, s_refuse, s_refuse, s_remove, s_size, s_list};

// Progressive's 2016 season, the part of it these seats touch.
static const char *kSeason2016 = R"({
  "data_group": "f1_career_progressive", "year": 2016,
  "teams": [
    { "slot": 3, "team": "constructor/red_bull_racing", "seats": ["game/daniel_ricciardo", "game/max_verstappen"] },
    { "slot": 4, "team": "constructor/mclaren", "seats": ["game/fernando_alonso", "person/jenson_button|game/jenson_button"] }
  ],
  "changes": [],
  "progressive": { "ratings_source": "official" }
})";

// Classic Seasons' 2022 grid, the part of it these seats touch, with its own looks.
static const char *kPackage2022 = R"({
  "title": "lng_cs_f1_2022", "entrant_list": "cs_f1_2022", "season": 2022,
  "teams": [
    { "key": "ferrari", "drivers": [ { "driver": "game/charles_leclerc", "number": 16, "look": null },
                                     { "driver": "game/carlos_sainz-jr", "number": 55, "look": null } ] },
    { "key": "williams", "drivers": [ { "driver": "game/alex_albon", "number": 23, "look": null },
                                      { "driver": "game/nicholas_latifi", "number": 6, "look": null } ] }
  ],
  "database": { "entrant_prefix": "cs2022-", "looks": { "williams/nicholas_latifi": "2022-cs_williams_nicholas_latifi" } },
  "leaderboard": "box"
})";

struct Run {
    swofty::fake::Core core{"teamsimplified.mods"};
    SwoftyApi api{};

    explicit Run(const char *storage)
    {
        g_storage = storage;
        api = core.api();
        api.storage = &kStorage;
        swofty::api = &api;
        core.run_register();
    }
};

static void the_2016_mclaren()
{
    Run run("test/storage/");
    auto seasons = run.core.layers("career.season");
    check(seasons.size() == 1, "one season layer: 2017 has no seats, so it gets none");
    if (seasons.size() != 1)
        return;
    check(seasons[0]->id == "swofty.progressive/2016", "on Progressive's 2016 season");
    check(seasons[0]->scope == SWOFTY_SCOPE_PROFILE, "for the profile that played");

    auto v = run.core.apply(*seasons[0], json::parse(kSeason2016));
    check(v.has_value(), "the season edit runs");
    if (!v)
        return;
    const json::Value &mclaren = (*v)["teams"][1];
    check(mclaren["seats"][1].as_string() == "person/stoffel_vandoorne|game/stoffel_vandoorne",
          "Vandoorne takes Button's McLaren seat");
    check(mclaren["seats"][0].as_string() == "game/fernando_alonso", "Alonso keeps his");
    check(mclaren["team"].as_string() == "constructor/mclaren", "the team stays Progressive's");
    check((*v)["teams"][0]["seats"][0].as_string() == "game/daniel_ricciardo", "Red Bull is untouched");
    check((*v)["progressive"]["ratings_source"].as_string() == "official", "Progressive's own data rides through");

    auto other = run.core.apply(*seasons[0], json::parse(R"({ "year": 2016, "teams": [ { "slot": 3,
        "team": "constructor/red_bull_racing", "seats": ["game/daniel_ricciardo", "game/max_verstappen"] } ] })"));
    check(other && (*other)["teams"].size() == 1, "a slot the season doesn't have is skipped, not added");
}

static void the_2022_williams()
{
    Run run("test/storage/");
    auto layers = run.core.layers("content.package");
    check(layers.size() == 2, "a layer a seat on the package");
    if (layers.size() != 2)
        return;
    check(layers[0]->id == "swofty.classic-seasons-2022/f1_2022", "on Classic Seasons' 2022 grid");
    check(layers[0]->scope == SWOFTY_SCOPE_PROFILE, "for the profile that played");
    json::Value v = json::parse(kPackage2022);
    for (const auto *l : layers) {
        auto next = run.core.apply(*l, v);
        check(next.has_value(), "the seat edit runs");
        if (next)
            v = *next;
    }
    const json::Value &williams = v["teams"][1];
    check(williams["drivers"][1]["driver"].as_string() == "swofty.classic-seasons/pascal_wehrlein",
          "Wehrlein takes Latifi's Williams seat");
    check(williams["drivers"][1]["number"].as_int() == 94, "with his number");
    check(williams["drivers"][1]["look"].is_null(), "and no look reference");
    check(v["database"]["looks"]["williams/pascal_wehrlein"].as_string() == "2016-cs_manor_pascal_wehrlein",
          "his look is in the package's own looks");
    check(v["database"]["looks"]["williams/nicholas_latifi"].as_string() == "2022-cs_williams_nicholas_latifi",
          "which keep theirs");
    check(williams["drivers"][0]["driver"].as_string() == "game/alex_albon", "Albon keeps his seat");
    check(v["teams"][0]["drivers"][0]["driver"].as_string() == "game/charles_leclerc" &&
              v["teams"][0]["drivers"][0]["number"].as_int() == 7,
          "a number alone changes only the number");
    check(v["database"]["entrant_prefix"].as_string() == "cs2022-" && v["leaderboard"].as_string() == "box",
          "the package's own data rides through");
}

static void no_seats_file()
{
    swofty::fake::Core core{"teamsimplified.mods"};
    SwoftyApi api = core.api();
    api.storage = &kStorage;
    g_storage = "test/no-such-folder/";
    swofty::api = &api;
    core.run_register();
    check(core.layers("career.season").empty() && core.layers("content.package").empty(), "no seats file, no layers");
}

int main()
{
    the_2016_mclaren();
    the_2022_williams();
    no_seats_file();
    return failures ? 1 : 0;
}

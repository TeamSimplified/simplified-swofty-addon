// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The seats layer against the SDK's fake core: it runs SWOFTY_REGISTER with test/storage as
// Simplified's folder in the game and applies the layers it records to Progressive's 2016 season.
// Run it from native/ with `build.sh test`.

#if defined(_WIN32)
#include <windows.h>
static const HMODULE kCoreStub = LoadLibraryW(L"swofty_core.dll");
#endif

#include "../src/progressive.cpp"

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
    check(seasons.size() == 1, "one layer: 2017 has no seats, so it gets none");
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

static void no_seats_file()
{
    swofty::fake::Core core{"teamsimplified.mods"};
    SwoftyApi api = core.api();
    api.storage = &kStorage;
    g_storage = "test/no-such-folder/";
    swofty::api = &api;
    core.run_register();
    check(core.layers("career.season").empty(), "no seats file, no layers");
}

int main()
{
    the_2016_mclaren();
    no_seats_file();
    return failures ? 1 : 0;
}

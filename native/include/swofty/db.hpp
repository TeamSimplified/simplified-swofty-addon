// SPDX-License-Identifier: MIT
#pragma once
#include <cstdint>
#include <string>
#include <string_view>
#include <swofty/mixin.hpp>

// Read access to the already-loaded game database. Keys are the dotted path, hashed with
// fnv1a64 (same keys the gamedb write operations use). Get a whole typed value, a single field
// of a known type, a row by its label, or iterate a table with a caller-owned cursor. String
// pointers returned here stay valid until the next db call on the same thread. Reads are
// available once the database has loaded; they return unavailable before then.

namespace swofty::db {

enum class Result { ok, late, unavailable, invalid };

enum class Tag : uint8_t {
    integer = SWOFTY_DB_INT,
    real = SWOFTY_DB_FLOAT,
    string = SWOFTY_DB_STRING,
    enumeration = SWOFTY_DB_ENUM,
    id = SWOFTY_DB_ID,
    boolean = SWOFTY_DB_BOOL,
};

using Value = SwoftyDbValue;
using Cursor = SwoftyDbCursor;

namespace detail {

inline const SwoftyDb *table()
{
    return api && api->version >= 5 ? api->db : nullptr;
}

inline Result result(int r)
{
    return r == SWOFTY_ASSET_OK ? Result::ok
           : r == SWOFTY_ASSET_LATE ? Result::late
           : r == SWOFTY_ASSET_UNAVAILABLE ? Result::unavailable
                                           : Result::invalid;
}

}

// The db-read library is usable (database loaded and the feature is on).
inline bool available()
{
    auto t = detail::table();
    return t && t->available();
}

// Whole typed value at a dotted key.
inline Result get(std::string_view key, Value *out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->get(std::string(key).c_str(), out));
}

// Whole typed value at a pre-hashed key.
inline Result get(uint64_t hash, Value *out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->get_hash(hash, out));
}

// Single field of a known type at a dotted key.
inline Result int_at(std::string_view key, int64_t *out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->int_at(std::string(key).c_str(), out));
}

inline Result float_at(std::string_view key, double *out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->float_at(std::string(key).c_str(), out));
}

inline Result string_at(std::string_view key, const char **out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->string_at(std::string(key).c_str(), out));
}

inline Result bool_at(std::string_view key, int32_t *out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->bool_at(std::string(key).c_str(), out));
}

// Position a cursor at the first row of a table.
inline Result open(std::string_view table, Cursor *cursor)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->open(std::string(table).c_str(), cursor));
}

// Position a cursor at a table row by its label.
inline Result row(std::string_view table, std::string_view label, Cursor *cursor)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->row(std::string(table).c_str(), std::string(label).c_str(), cursor));
}

// Advance a cursor to the next row; late when the table is exhausted.
inline Result step(Cursor *cursor)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->step(cursor));
}

// Read one field of the cursor's current row.
inline Result cell(const Cursor *cursor, std::string_view field, Value *out)
{
    auto t = detail::table();
    if (!t)
        return Result::unavailable;
    return detail::result(t->cell(cursor, std::string(field).c_str(), out));
}

// Label of the cursor's current row, or nullptr.
inline const char *row_name(const Cursor *cursor)
{
    auto t = detail::table();
    return t ? t->row_name(cursor) : nullptr;
}

inline const char *name(Result r)
{
    return r == Result::ok ? "ok" : r == Result::late ? "late" : r == Result::unavailable ? "unavailable" : "invalid";
}

}

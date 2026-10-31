// SPDX-License-Identifier: MIT
#pragma once
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <initializer_list>
#include <string_view>
#include <type_traits>
#include <utility>
#include <vector>
#include <swofty_api.h>

namespace swofty {

using Context = SwoftyContext;
inline const SwoftyApi *api;

template <class... A> void log(const char *fmt, A... a)
{
    if (api)
        api->log(fmt, a...);
}

template <size_t N> struct name {
    char s[N]{};
    constexpr name(const char (&v)[N])
    {
        for (size_t i = 0; i < N; i++)
            s[i] = v[i];
    }
};

enum class kind { func, site, call, vtable, data };

namespace detail {

struct want {
    const char *id;
    uintptr_t *slot;
    want *next;
};

struct node {
    SwoftyMixin m;
    node *next;
};

inline want *wants;
inline node *mixins;

template <class T> void append(T *&head, T *item)
{
    T **tail = &head;
    while (*tail)
        tail = &(*tail)->next;
    *tail = item;
}

inline uintptr_t wanted(const char *id, uintptr_t *slot)
{
    append(wants, new want{id, slot, nullptr});
    return 0;
}

template <name N> struct bound {
    static inline uintptr_t addr = wanted(N.s, &addr);
};

struct mixin {
    mixin(const char *target, uint32_t kind, int32_t priority, void *handler, void **orig)
    {
        append(mixins, new node{{target, kind, priority, handler, orig}, nullptr});
    }
};

template <class T> constexpr uint32_t setting_type()
{
    static_assert(std::is_arithmetic_v<T> || std::is_same_v<T, std::string_view>,
                  "a setting is bool (toggle), a number (slider) or std::string_view (choice)");
    if constexpr (std::is_same_v<T, bool>)
        return SWOFTY_SETTING_TOGGLE;
    else if constexpr (std::is_arithmetic_v<T>)
        return SWOFTY_SETTING_SLIDER;
    else
        return SWOFTY_SETTING_CHOICE;
}

template <class T> T from_value(const SwoftySettingValue &v)
{
    if constexpr (std::is_same_v<T, bool>)
        return v.toggle != 0;
    else if constexpr (std::is_integral_v<T>)
        return static_cast<T>(v.slider < 0 ? v.slider - 0.5 : v.slider + 0.5);
    else if constexpr (std::is_floating_point_v<T>)
        return static_cast<T>(v.slider);
    else
        return v.choice ? T(v.choice) : T();
}

inline bool settings_api(const SwoftyApi *a)
{
    return a && a->version >= 3 && a->setting && a->watch_setting;
}

struct declared_setting {
    const char *id;
    uint32_t type;
    declared_setting *next;
};

struct watcher {
    const char *const *id;
    void (*fn)(const SwoftySettingValue &);
    watcher *next;
};

inline declared_setting *settings;
inline watcher *watchers;

struct watch {
    watch(const char *const *id, void (*fn)(const SwoftySettingValue &))
    {
        append(watchers, new watcher{id, fn, nullptr});
    }
};

inline void init_settings(const SwoftyApi *a)
{
    static const char *const kinds[] = {"?", "toggle", "slider", "choice"};
    if (!settings_api(a)) {
        if (settings)
            a->log("addon settings: swofty_core offers no settings API (v3); every setting keeps its code default");
        return;
    }
    for (declared_setting *s = settings; s; s = s->next) {
        SwoftySettingValue v{};
        if (!a->setting(s->id, s->type, &v))
            a->log("addon settings: %s is not a %s option of this addon in swofty.json; the code default is used", s->id,
                   kinds[s->type]);
    }
    for (watcher *w = watchers; w; w = w->next)
        a->watch_setting(
            *w->id, [](const char *, const SwoftySettingValue *v, void *u) { static_cast<watcher *>(u)->fn(*v); }, w);
}

inline int init(const SwoftyApi *a, bool (*check)(), std::nullptr_t = nullptr)
{
    if (!a || a->version < 2 || !a->resolve || !a->register_mixins) {
        if (a && a->log)
            a->log("mixin addon: needs the swofty_core mixin API (v2), loader offers v%u", a->version);
        return 0;
    }
    api = a;
    bool ok = true;
    for (want *w = wants; w; w = w->next)
        ok &= (*w->slot = a->resolve(w->id)) != 0;
    if (!ok || (check && !check()))
        return 0;
    std::vector<SwoftyMixin> list;
    for (node *n = mixins; n; n = n->next)
        list.push_back(n->m);
    if (!a->register_mixins(list.data(), list.size()))
        return 0;
    init_settings(a);
    return 1;
}

struct vt_copy {
    void **src, **vt;
    size_t count;
    std::vector<std::pair<size_t, void *>> slots;
    vt_copy *next;
};
inline vt_copy *vt_copies;
inline std::atomic_flag vt_lock;

}

template <name N, kind K> struct binding {
    static constexpr const char *id = N.s;
    static constexpr swofty::kind binding_kind = K;
    static uintptr_t address() { return detail::bound<N>::addr; }
};

template <name N, class F> struct fn;
template <name N, class R, class... A> struct fn<N, R(A...)> : binding<N, kind::func> {
    using type = R(A...);
    type *get() const { return reinterpret_cast<type *>(detail::bound<N>::addr); }
    R operator()(A... a) const { return get()(a...); }
};
template <name N, class R, class... A> struct fn<N, R(A..., ...)> : binding<N, kind::func> {
    using type = R(A..., ...);
    type *get() const { return reinterpret_cast<type *>(detail::bound<N>::addr); }
    template <class... V> R operator()(A... a, V... v) const { return get()(a..., v...); }
};

template <name N> struct address : binding<N, kind::func> {};
template <name N> struct site : binding<N, kind::site> {};
template <name N> struct call : binding<N, kind::call> {};

template <name N, class T> struct data : binding<N, kind::data> {
    T *get() const { return reinterpret_cast<T *>(detail::bound<N>::addr); }
    T &operator*() const { return *get(); }
};

template <name N, size_t Slots> struct vtable : binding<N, kind::vtable> {
    static constexpr size_t slots = Slots;
    void **get() const { return reinterpret_cast<void **>(detail::bound<N>::addr); }
};

inline void **subclass(void *obj, std::initializer_list<std::pair<size_t, void *>> slots, size_t count = 32)
{
    void **src = *static_cast<void ***>(obj);
    std::vector<std::pair<size_t, void *>> want(slots);
    while (detail::vt_lock.test_and_set(std::memory_order_acquire)) {
    }
    detail::vt_copy *c = detail::vt_copies;
    while (c && c->vt != src && (c->src != src || c->count != count || c->slots != want))
        c = c->next;
    if (!c) {
        auto **storage = new void *[count + 1];
        memcpy(storage, src - 1, (count + 1) * sizeof(void *));
        for (auto [i, f] : want)
            storage[1 + i] = f;
        c = new detail::vt_copy{src, storage + 1, count, std::move(want), detail::vt_copies};
        detail::vt_copies = c;
    }
    detail::vt_lock.clear(std::memory_order_release);
    *static_cast<void ***>(obj) = c->vt;
    return c->vt;
}

template <class T> T *fields(const void *obj)
{
    static_assert(std::is_trivially_default_constructible_v<T> && std::is_trivially_destructible_v<T>);
    return static_cast<T *>(api->fields(obj, sizeof(T)));
}

inline void release_fields(const void *obj)
{
    api->fields(obj, 0);
}

template <class T> struct setting {
    using type = T;
    const char *id;
    T fallback;

    setting(const char *id, T fallback) : id(id), fallback(fallback)
    {
        detail::append(detail::settings, new detail::declared_setting{id, detail::setting_type<T>(), nullptr});
    }

    T get() const
    {
        SwoftySettingValue v{};
        if (detail::settings_api(api) && api->setting(id, detail::setting_type<T>(), &v))
            return detail::from_value<T>(v);
        return fallback;
    }

    operator T() const { return get(); }

    friend bool operator==(const setting &s, T v) { return s.get() == v; }
};

}

#define SWOFTY_CAT_(a, b) a##b
#define SWOFTY_CAT(a, b) SWOFTY_CAT_(a, b)
#define SWOFTY_NS SWOFTY_CAT(swofty_mixin_, __LINE__)
#define SWOFTY_KIND_(target, want)                                                                                    \
    static_assert(std::remove_cvref_t<decltype(::swofty::game::target)>::binding_kind == ::swofty::kind::want,       \
                  #target " is not a " #want " binding")
#define SWOFTY_REG_(target, kind, prio, handler, orig)                                                                \
    const ::swofty::detail::mixin reg{::swofty::game::target.id, kind, prio, (void *)(handler), (void **)(orig)}

#define SWOFTY_FN_(kind, want, prio, target, R, ...)                                                                  \
    namespace {                                                                                                       \
    namespace SWOFTY_NS {                                                                                             \
    SWOFTY_KIND_(target, want);                                                                                       \
    R (*orig)(__VA_ARGS__);                                                                                           \
    R handler(__VA_ARGS__);                                                                                           \
    SWOFTY_REG_(target, kind, prio, &handler, &orig);                                                                 \
    }                                                                                                                 \
    }                                                                                                                 \
    R SWOFTY_NS::handler(__VA_ARGS__)

#define SWOFTY_WRAP_(flags, prio, target, R, ...)                                                                     \
    static_assert(std::is_same_v<::swofty::game::target##_t, R(__VA_ARGS__)>,                                        \
                  #target ": handler signature differs from the binding");                                            \
    SWOFTY_FN_(SWOFTY_MIXIN_WRAP | (flags), func, prio, target, R, __VA_ARGS__)

#define SWOFTY_WRAP(target, R, ...) SWOFTY_WRAP_(0, 0, target, R, __VA_ARGS__)
#define SWOFTY_WRAP_PRIORITY(prio, target, R, ...) SWOFTY_WRAP_(0, prio, target, R, __VA_ARGS__)
#define SWOFTY_OVERWRITE(prio, target, R, ...) SWOFTY_WRAP_(SWOFTY_MIXIN_OVERWRITE, prio, target, R, __VA_ARGS__)

#define SWOFTY_VTABLE_PRIORITY(prio, target, slot, R, ...)                                                            \
    static_assert((slot) < std::remove_cvref_t<decltype(::swofty::game::target)>::slots, #target ": slot out of range"); \
    SWOFTY_FN_(SWOFTY_MIXIN_WITH_ARG(SWOFTY_MIXIN_VTABLE, slot), vtable, prio, target, R, __VA_ARGS__)
#define SWOFTY_VTABLE(target, slot, R, ...) SWOFTY_VTABLE_PRIORITY(0, target, slot, R, __VA_ARGS__)

#define SWOFTY_REDIRECT(target, R, ...) SWOFTY_FN_(SWOFTY_MIXIN_REDIRECT, call, 0, target, R, __VA_ARGS__)

#define SWOFTY_SITE_PRIORITY(prio, target, param)                                                                     \
    namespace {                                                                                                       \
    namespace SWOFTY_NS {                                                                                             \
    SWOFTY_KIND_(target, site);                                                                                       \
    void handler(param);                                                                                              \
    void entry(SwoftyContext *c) { handler(*c); }                                                                     \
    SWOFTY_REG_(target, SWOFTY_MIXIN_SITE, prio, &entry, nullptr);                                                    \
    }                                                                                                                 \
    }                                                                                                                 \
    void SWOFTY_NS::handler(param)
#define SWOFTY_SITE(target, param) SWOFTY_SITE_PRIORITY(0, target, param)

#define SWOFTY_PATCH(target, ...)                                                                                     \
    namespace {                                                                                                       \
    namespace SWOFTY_NS {                                                                                             \
    SWOFTY_KIND_(target, site);                                                                                       \
    const uint8_t bytes[] = {__VA_ARGS__};                                                                            \
    SWOFTY_REG_(target, SWOFTY_MIXIN_WITH_ARG(SWOFTY_MIXIN_PATCH, sizeof bytes), 0, bytes, nullptr);                  \
    }                                                                                                                 \
    }

#define SWOFTY_ON_LOAD                                                                                                \
    namespace {                                                                                                       \
    namespace SWOFTY_NS {                                                                                             \
    void handler();                                                                                                   \
    const ::swofty::detail::mixin reg{nullptr, SWOFTY_MIXIN_ON_LOAD, 0, reinterpret_cast<void *>(&handler), nullptr}; \
    }                                                                                                                 \
    }                                                                                                                 \
    void SWOFTY_NS::handler()

#define SWOFTY_SETTING(type, id, fallback) inline const ::swofty::setting<type> id{#id, fallback}

#define SWOFTY_ON_CHANGE(setting_name, param)                                                                         \
    namespace {                                                                                                       \
    namespace SWOFTY_NS {                                                                                             \
    using swofty_setting_t = std::remove_cvref_t<decltype(setting_name)>::type;                                       \
    void handler(param);                                                                                              \
    const ::swofty::detail::watch reg{&(setting_name).id, [](const SwoftySettingValue &v) {                            \
        if (v.type == ::swofty::detail::setting_type<swofty_setting_t>())                                             \
            handler(::swofty::detail::from_value<swofty_setting_t>(v));                                               \
    }};                                                                                                               \
    }                                                                                                                 \
    }                                                                                                                 \
    void SWOFTY_NS::handler(param)

#define SWOFTY_ADDON(...)                                                                                             \
    extern "C" __declspec(dllexport) const uint32_t SwoftyAddonApi = SWOFTY_API_VERSION;                              \
    extern "C" __declspec(dllexport) int SwoftyAddonInit(const SwoftyApi *a)                                          \
    {                                                                                                                 \
        return ::swofty::detail::init(a, __VA_OPT__(&__VA_ARGS__, ) nullptr);                                         \
    }

#define SWOFTY_MOD(...) SWOFTY_ADDON(__VA_ARGS__)

// Stands in for swofty_core.dll in the Windows test: the SDK looks for this export before it uses
// the registry the fake core hands it.
extern "C" __declspec(dllexport) void SwoftyCoreRegistry() {}

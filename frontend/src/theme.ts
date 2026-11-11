/** Follows the operating system's light/dark setting and keeps following it while the app is open. */
const query = "(prefers-color-scheme: dark)";

export function applyTheme(dark: boolean) {
  const root = document.documentElement;
  root.classList.toggle("dark", dark);
  root.style.colorScheme = dark ? "dark" : "light";
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", dark ? "#0f141c" : "#f5f7fb");
}

export function watchSystemTheme(): () => void {
  if (typeof window.matchMedia !== "function") return () => {};
  const media = window.matchMedia(query);
  const update = () => applyTheme(media.matches);
  update();
  media.addEventListener("change", update);
  return () => media.removeEventListener("change", update);
}

/** Runs before first paint (inlined in <head>) so a dark system never flashes white. */
export const themeBootScript = `try{var d=matchMedia("${query}").matches;document.documentElement.classList.toggle("dark",d);document.documentElement.style.colorScheme=d?"dark":"light"}catch(e){}`;

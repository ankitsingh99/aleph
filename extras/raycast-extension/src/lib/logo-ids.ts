// Which logo stands for an OS, a GPU or a model's maker — ids of the bundled
// PNGs (assets/logos, tools/logogen), each brand's site and its name. No
// Raycast imports: the list icons (logos.ts) and the SVG cards (dash.ts,
// machines-view.ts) share it, and plain node tests it.

export interface LogoId {
  /** the PNG's id under assets/logos (`ubuntu`, `nvidia`, …) */
  id: string;
  /** the site whose favicon stands in when there's no PNG */
  site: string;
  name: string;
}

/** Distribution / OS → its logo, most specific first. */
const OS_LOGOS: [RegExp, string, string, string][] = [
  [/ubuntu/i, "ubuntu", "https://ubuntu.com", "Ubuntu"],
  [/pop!?_?os/i, "popos", "https://system76.com", "Pop!_OS"],
  [/mint/i, "linuxmint", "https://linuxmint.com", "Linux Mint"],
  [/kali/i, "kalilinux", "https://www.kali.org", "Kali Linux"],
  [/raspbian|raspberry/i, "raspberrypi", "https://www.raspberrypi.com", "Raspberry Pi OS"],
  [/debian/i, "debian", "https://www.debian.org", "Debian"],
  [/fedora/i, "fedora", "https://fedoraproject.org", "Fedora"],
  [/rocky/i, "rockylinux", "https://rockylinux.org", "Rocky Linux"],
  [/alma/i, "almalinux", "https://almalinux.org", "AlmaLinux"],
  [/centos/i, "centos", "https://www.centos.org", "CentOS"],
  [/red ?hat|rhel/i, "redhat", "https://www.redhat.com", "Red Hat"],
  [/oracle linux/i, "oraclelinux", "https://www.oracle.com/linux", "Oracle Linux"],
  [/endeavour/i, "endeavouros", "https://endeavouros.com", "EndeavourOS"],
  [/manjaro/i, "manjaro", "https://manjaro.org", "Manjaro"],
  [/\barch( linux)?\b/i, "archlinux", "https://archlinux.org", "Arch Linux"],
  [/gentoo/i, "gentoo", "https://www.gentoo.org", "Gentoo"],
  [/void/i, "voidlinux", "https://voidlinux.org", "Void Linux"],
  [/opensuse/i, "opensuse", "https://www.opensuse.org", "openSUSE"],
  [/suse/i, "suse", "https://www.suse.com", "SUSE"],
  [/alpine/i, "alpinelinux", "https://alpinelinux.org", "Alpine"],
  [/nixos/i, "nixos", "https://nixos.org", "NixOS"],
  [/elementary/i, "elementary", "https://elementary.io", "elementary OS"],
  [/amazon linux/i, "amazonlinux", "https://aws.amazon.com", "Amazon Linux"],
  [/mac ?os|darwin|os x/i, "apple", "https://www.apple.com", "macOS"],
  [/windows|wsl/i, "windows", "https://www.microsoft.com", "Windows"],
  [/freebsd/i, "freebsd", "https://www.freebsd.org", "FreeBSD"],
  [/openbsd/i, "openbsd", "https://www.openbsd.org", "OpenBSD"],
  [/linux/i, "linux", "https://www.kernel.org", "Linux"],
];

/** The OS's logo from any of `os`, `os_name`, `platform` (`Ubuntu 24.04.3 LTS`, `Darwin arm64`, …). */
export function osId(...names: (string | null | undefined)[]): LogoId | undefined {
  const text = names.filter(Boolean).join(" ");
  if (!text) return undefined;
  const hit = OS_LOGOS.find(([re]) => re.test(text));
  return hit ? { id: hit[1], site: hit[2], name: hit[3] } : undefined;
}

/** The GPU vendor's logo (NVIDIA, AMD, Intel, Apple) from a GPU name or vendor. */
export function gpuId(...names: (string | null | undefined)[]): LogoId | undefined {
  const text = names.filter(Boolean).join(" ");
  if (/nvidia|geforce|rtx|gtx|tesla|quadro|a100|h100|h200|b200|gh200|l40|a6000|a10g/i.test(text))
    return { id: "nvidia", site: "https://www.nvidia.com", name: "NVIDIA" };
  if (/\bamd\b|radeon|instinct|\bmi\d{3}\b/i.test(text)) return { id: "amd", site: "https://www.amd.com", name: "AMD" };
  if (/\bintel\b|\barc\b/i.test(text)) return { id: "intel", site: "https://www.intel.com", name: "Intel" };
  if (/apple/i.test(text)) return { id: "apple", site: "https://www.apple.com", name: "Apple" };
  return undefined;
}

/** `NVIDIA GeForce RTX 4090` → `RTX 4090`; `AMD Instinct MI300X` → `MI300X` (the part people say). */
export function shortGpu(name: string): string {
  return name
    .replace(/^(nvidia|amd|intel|apple)\s+/i, "")
    .replace(/^(geforce|quadro|tesla|radeon( pro)?|instinct|arc)\s+/i, "")
    .replace(/\s+(pcie|sxm\d?)(-\d+gb)?$/i, "")
    .replace(/\s+\d+\s*gb$/i, "")
    .trim();
}

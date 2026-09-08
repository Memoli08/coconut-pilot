Name: coconut-pilot
Version: 0.1.0
Release: 1%{?dist}
Summary: Your keyboard, your shortcuts.
License: MIT
Source0: %{name}-%{version}.tar.gz
BuildRequires: cargo
BuildRequires: rust
BuildRequires: gcc
BuildRequires: pkgconfig(gio-2.0)
Requires: systemd
Requires: sudo
Recommends: libnotify

%description
Interactive Copilot key configuration with applications, websites and commands.

%prep
%autosetup

%build
cargo build --release --locked --offline

%check
cargo test --locked --offline

%install
install -Dm755 target/release/coconut %{buildroot}%{_bindir}/coconut

%files
%license LICENSE
%doc README.md
%{_bindir}/coconut

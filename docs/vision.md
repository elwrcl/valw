> Original product vision, kept as written. The project was renamed from `snapr` to `valw`; phase designs live in `docs/superpowers/specs/`.

# SPEC.md: macOS tarzı ekran görüntüsü + zoom aracı (niri / Wayland / NixOS)

> Bu dosyayı projenin köküne `SPEC.md` (veya `CLAUDE.md`) olarak koy ve Claude Code'a
> "SPEC.md'yi oku, Faz 1'den başla" de. Proje adı şimdilik `snapr`, istediğin gibi değiştir.

## 1. Amaç

macOS'taki **Cmd+Shift+3/4/5** deneyimini niri üzerinde birebir hissettiren, Rust ile yazılmış tek bir araç:

- Bölge / pencere / tam ekran yakalama
- macOS'taki gibi **sağ altta beliren küçük önizleme** (tıkla → düzenle, sürükle → bırak, kaydır → kapat)
- Basit bir **düzenleyici** (ok, kutu, metin, kalem, bulanıklaştırma, kırpma)
- **Zoom / büyüteç modu** (woomer benzeri: zoom, pan, flashlight)
- Cmd+Shift+5 benzeri **araç çubuğu**

## 2. Hedef ortam ve kısıtlar

- **Compositor:** niri (öncelikli). Diğer wlroots tabanlı compositor'larda çalışması bonus, zorunlu değil.
- **Shell:** Noctalia. Doğrudan bir Noctalia API'sine bağımlı olma; entegrasyon **CLI + standart Wayland protokolleri + freedesktop bildirimleri** üzerinden olsun, böylece Noctalia'nın iç yapısı değişse de araç çalışır.
- **Dağıtım:** NixOS. Proje bir **flake** olarak paketlenmeli.
- **Dil:** Rust (stable). X11 desteği yok, sadece Wayland.

> ⚠️ Claude Code için: protokol ve IPC desteğini **varsayma, doğrula.** Kodlamaya başlamadan önce
> kullanıcının makinesinde `niri --version`, `niri msg --help`, `wayland-info` çıktılarına bak ve
> hangi protokollerin sunulduğunu raporla.

## 3. Teknik yaklaşım

| Konu | Tercih | Not |
|---|---|---|
| Wayland istemcisi | `smithay-client-toolkit` (SCTK) + `wayland-client` | |
| Overlay yüzeyleri | `wlr-layer-shell` | Seçim ekranı, önizleme, zoom modu hepsi layer-shell yüzeyi |
| Ekran yakalama | `ext-image-copy-capture-v1` varsa onu, yoksa `wlr-screencopy-unstable-v1` | İkisini de bir trait arkasına soy |
| Pencere bilgisi | `niri-ipc` crate'i ile niri IPC | Pencere listesi, odaklı pencere, monitör bilgisi. Pencere geometrisi IPC'den alınamıyorsa pencere yakalamada niri'nin kendi `screenshot-window` aksiyonuna düş |
| Çizim | `tiny-skia` (yazılımsal) | Gerekirse sonradan `wgpu`'ya geçilebilir; zoom modunda performans yetmezse ilk aday orası |
| Metin | `cosmic-text` veya `fontdue` | |
| Pano | `wl-clipboard-rs` | PNG olarak kopyala |
| Bildirim | `notify-rust` | Önizleme penceresi yerine fallback olarak |
| Görüntü | `image` crate'i (PNG kaydetme) | |
| Config | `serde` + `toml`, `~/.config/snapr/config.toml` | |
| CLI | `clap` | |

**HiDPI / kesirli ölçekleme** en baştan düşünülmeli: `wp-fractional-scale-v1` ve `wp-viewporter` kullan, yakalanan görüntü fiziksel piksel çözünürlüğünde olsun.

**Çoklu monitör:** Seçim modu tüm monitörlerde aynı anda açılmalı; bölge tek monitörle sınırlı olabilir (v1 için yeterli).

## 4. Özellikler

### 4.1 Yakalama modları (CLI alt komutları)

```
snapr region        # Cmd+Shift+4: sürükleyerek bölge seç
snapr window        # Cmd+Shift+4 + Space: üzerine gelinen pencere vurgulanır, tıkla → yakala
snapr screen        # Cmd+Shift+3: odaklı monitör (veya --all)
snapr toolbar       # Cmd+Shift+5: araç çubuğu
snapr zoom          # woomer benzeri zoom modu
snapr edit <dosya>  # düzenleyiciyi doğrudan aç
```

Ortak bayraklar: `--clipboard-only`, `--no-preview`, `--delay <sn>`, `--output <yol>`, `--cursor`.

### 4.2 Bölge seçimi (macOS davranışı)

- Ekran donar (yakalanmış kare arka plan olarak gösterilir), hafif karartılır.
- İmleç yanında **koordinat / boyut** göstergesi (örn. `640 × 480`).
- Sürüklerken:
  - **Shift**: sadece bir eksende genişlet
  - **Alt**: merkezden genişlet
  - **Space basılı tut**: seçimi taşı
- **Space (sürüklemeden önce)**: pencere moduna geç (macOS'taki kamera ikonu mantığı).
- **Esc**: iptal.
- Bırakınca yakala.

### 4.3 Pencere modu

- İmlecin altındaki pencere mavi tonla vurgulanır.
- Tıkla → yakala. Opsiyonel: pencere çevresine gölge ekleme (config ile, macOS'taki gibi, varsayılan kapalı).

### 4.4 Sağ alt önizleme (macOS'un "floating thumbnail"ı)

- Yakalamadan sonra odaklı monitörün sağ alt köşesinde, layer-shell ile küçük bir önizleme belirir (kayarak girer).
- Varsayılan **5 sn** sonra kaybolur ve dosya kaydedilir (süre config'den).
- Etkileşimler:
  - **Sol tık** → düzenleyicide aç
  - **Sağ tık** → küçük menü: Kaydet / Panoya kopyala / Klasörde göster / Sil / Düzenle
  - **Sağa kaydır/sürükle** → hemen kaydet ve kapat
  - **Dışarı sürükle** → `wl_data_device` ile sürükle-bırak (dosyayı başka bir uygulamaya bırakabilmek)
  - Üzerine gelince zamanlayıcı durur
- Önizleme devre dışıysa `notify-send` benzeri bir bildirim gönder (resim + "Düzenle" aksiyonu). Noctalia bildirimleri bunu gösterecektir.

### 4.5 Düzenleyici

Normal bir pencere (xdg-toplevel). Araçlar:

- Ok, dikdörtgen, elips, çizgi, serbest kalem, işaretleyici (yarı saydam)
- Metin (font boyutu, renk)
- Bulanıklaştır / pikselleştir (hassas bilgileri gizlemek için)
- Numaralı işaret (1, 2, 3... baloncukları)
- Kırp
- Renk paleti + kalınlık
- Geri al / yinele (Ctrl+Z / Ctrl+Shift+Z)
- Ctrl+C panoya kopyala, Ctrl+S kaydet, Esc kapat

> Alternatif: v1'de düzenleyiciyi yazmak yerine **Satty**'yi çağırmak mümkün. Faz planına bak.

### 4.6 Araç çubuğu (Cmd+Shift+5)

Ekranın altında ortalanmış yüzen bir çubuk:

`[Tam ekran] [Pencere] [Bölge] | [Zoom] | Seçenekler ▾ | [Yakala]`

Seçenekler: kayıt yeri (Masaüstü / Resimler / Pano / Özel), zamanlayıcı (yok / 5 sn / 10 sn), imleci göster, önizlemeyi göster, son seçimi hatırla.

> Ekran kaydı (video) **v1 kapsamı dışında**. İleride `gpu-screen-recorder` veya `wf-recorder` çağrılarak eklenebilir.

### 4.7 Zoom modu (woomer'dan esinlenme)

- Ekranın anlık görüntüsü üzerinde tam ekran layer-shell yüzeyi.
- Fare tekerleği: imlecin olduğu noktaya doğru zoom
- Sol tık + sürükle: pan
- `f`: flashlight aç/kapat; `Ctrl + tekerlek`: flashlight yarıçapı
- `0`: sıfırla, `Esc`/`q`: çık
- Yumuşak (animasyonlu) zoom ve pan
- **Bonus:** zoom modundayken `c` tuşu → o anki görünümü yakala ve önizleme akışına gönder

> ⚠️ Lisans: woomer'dan kod kopyalanacaksa önce **lisansını kontrol et** ve uyumluysa atıf ekle.
> Tercih edilen yol: davranışı referans alıp kendi implementasyonunu yazmak. Böylece aynı yakalama ve
> çizim altyapısı paylaşılır, woomer'ın ayrı render bağımlılıklarına ihtiyaç kalmaz.

## 5. Config örneği

```toml
# ~/.config/snapr/config.toml
[save]
directory = "~/Pictures/Screenshots"
filename = "Screenshot %Y-%m-%d at %H.%M.%S.png"
copy_to_clipboard = true

[preview]
enabled = true
timeout_secs = 5
corner = "bottom-right"   # top-left | top-right | bottom-left | bottom-right
size = 220                # px, uzun kenar

[capture]
show_cursor = false
window_shadow = false

[editor]
backend = "builtin"       # builtin | satty
default_color = "#ff3b30"
stroke_width = 4

[zoom]
scroll_step = 1.15
flashlight_radius = 180
```

## 6. niri entegrasyonu

```kdl
binds {
    Mod+Shift+3 { spawn "snapr" "screen"; }
    Mod+Shift+4 { spawn "snapr" "region"; }
    Mod+Shift+5 { spawn "snapr" "toolbar"; }
    Mod+Z       { spawn "snapr" "zoom"; }
}
```

Layer-shell yüzeylerinin namespace'leri sabit ve belgelenmiş olsun (`snapr-overlay`, `snapr-preview`, `snapr-zoom`), böylece gerekirse niri'de `layer-rule` ile özelleştirilebilir.

## 7. NixOS paketleme

- `flake.nix` içinde:
  - `packages.<system>.default`: `rustPlatform.buildRustPackage` (veya `crane`)
  - `devShells.<system>.default`: rust toolchain, `pkg-config`, `wayland`, `libxkbcommon`, `fontconfig`, `wayland-protocols`
  - `homeManagerModules.default`: `programs.snapr.enable` + `programs.snapr.settings` (TOML'a çevrilir)
- Runtime'da gereken kütüphaneler için doğru `buildInputs`/`nativeBuildInputs` ve gerekirse `wrapProgram` ile `LD_LIBRARY_PATH`.
- `nix build` ve `nix run . -- region` kutudan çıktığı gibi çalışmalı.

## 8. Mimari

```
src/
  main.rs            # clap CLI, alt komut yönlendirme
  config.rs
  capture/
    mod.rs           # trait Capturer
    ext_copy.rs      # ext-image-copy-capture-v1
    wlr_screencopy.rs
  niri.rs            # niri-ipc sarmalayıcı (pencereler, monitörler, odak)
  overlay/
    region.rs        # bölge + pencere seçimi
    toolbar.rs
    zoom.rs
  preview.rs         # sağ alt önizleme
  editor/
    mod.rs
    tools.rs         # ok, kutu, metin, blur...
    history.rs       # undo/redo
  output.rs          # kaydetme, pano, bildirim
  render.rs          # tiny-skia yardımcıları
```

- Her mod kendi başına çalışan kısa ömürlü bir süreç olabilir (daemon gerekmez). Önizleme yakalamadan sonra aynı süreçte yaşar ve zamanlayıcı bitince çıkar.
- İki yakalama aynı anda başlatılırsa ikincisi birincinin bitmesini bekler veya iptal eder (lock dosyası: `$XDG_RUNTIME_DIR/snapr.lock`).

## 9. Faz planı

**Faz 0: Keşif**
Kullanıcının sisteminde protokol desteğini tespit et, raporla. Boş SCTK + layer-shell yüzeyi açıp kapatan minimal örnek. `flake.nix` + devShell.

**Faz 1: Çekirdek yakalama**
`snapr screen` ve `snapr region` (temel sürükle-seç, Esc iptal). PNG kaydetme + panoya kopyalama. Config dosyası.

**Faz 2: Önizleme**
Sağ alt önizleme, zamanlayıcı, tık → (şimdilik) Satty'de aç, kaydır → kapat. Bildirim fallback'i.

**Faz 3: Pencere modu + seçim inceliği**
niri IPC ile pencere vurgulama, Space ile mod değiştirme, Shift/Alt/Space modifikatörleri, boyut göstergesi, HiDPI doğruluğu.

**Faz 4: Zoom modu**
woomer benzeri zoom/pan/flashlight, zoomdan yakalama.

**Faz 5: Yerleşik düzenleyici**
Araçlar, undo/redo, kırpma, blur. `editor.backend = "builtin"` varsayılan olur.

**Faz 6: Araç çubuğu + cila**
Cmd+Shift+5 çubuğu, zamanlayıcı, sürükle-bırak, home-manager modülü, README.

Her fazın sonunda: `cargo clippy -- -D warnings`, `cargo fmt --check`, `nix build` geçmeli ve fazın özelliği niri üzerinde elle test edilmiş olmalı.

## 10. Kabul kriterleri

- 2 monitörlü, biri kesirli ölçekli (örn. 1.25) kurulumda bölge seçimi piksel-doğru yakalar.
- Kısayola basmaktan seçim ekranının görünmesine kadar geçen süre < 150 ms hissi verir.
- Önizleme odak çalmaz (klavye odağı almaz), sadece üzerine tıklanınca etkileşir.
- Hiçbir mod ekranda takılı kalan bir overlay bırakmaz; çökme durumunda bile süreç ölünce yüzey kaybolur.
- Noctalia'nın bildirimleriyle ve barıyla çakışmaz (layer z-sırası doğru).

## 11. Kapsam dışı (v1)

- X11, GNOME, KDE desteği
- Video / GIF kaydı
- OCR, bulut yükleme
- Kaydırmalı (scrolling) ekran görüntüsü

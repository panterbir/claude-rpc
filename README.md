# claude-rpc

Claude kullanımını Discord Rich Presence olarak gösteren, tek binary'li (`claude-rpc.exe`, ~350 KB) Windows uygulaması.
Rust, async yok, GUI yok; tek `std::thread` döngüsü. Boştayken ~1 MB working set / ~0,25 MB private working set.

Gösterilen:

| Alan | Değer |
|---|---|
| details | proje adı (cwd'nin son klasörü) |
| state | `<branch> · Aktif` / `<branch> · Boşta` (git yoksa sadece `Aktif`/`Boşta`) |
| timestamp | oturumun başlangıcı (proje değişirse sıfırlanır) |
| large image | `claude` · small image: `active` / `idle` |

## Kurulum

1. **Discord uygulaması** — <https://discord.com/developers/applications> → *New Application* → adı ne yazarsan Discord'da "… oynuyor" olarak görünür (ör. `Claude`). *General Information* sayfasındaki **Application ID**'yi kopyala.
2. **Asset'ler** — aynı uygulamada *Rich Presence → Art Assets* → şu anahtarlarla görsel yükle (min. 512×512, adlar küçük harf):
   - `claude` (büyük görsel)
   - `active` (küçük, aktifken)
   - `idle` (küçük, boştayken)

   Asset'ler Discord'da yayılması birkaç dakika sürebilir. Farklı adlar kullanırsan `config.toml`'dan değiştir.
3. **Application ID (isteğe bağlı)** — hiçbir şey yapmazsan uygulama, içine gömülü varsayılan uygulamayı kullanır (Discord'da "AI Coding" olarak görünür, görseller hazır). Application ID gizli bir bilgi değil, herkese açık bir tanımlayıcıdır. Kendi uygulamanı/görsellerini kullanmak istersen (1. ve 2. adımlar) biri yeterli:
   - `%LOCALAPPDATA%\claude-rpc\config.toml` dosyasına `client_id = "123..."` (örnek: [`config.example.toml`](config.example.toml)), veya
   - kullanıcı ortam değişkeni `CLAUDE_RPC_CLIENT_ID` (config'i ezer).

   Ayar verilmezse gömülü varsayılan ID kullanılır; bu yüzden herkes ekstra kurulum yapmadan çalıştırabilir.
4. **Derle** (Rust + MSVC Build Tools gerekir) ya da `dist\claude-rpc.exe`'yi kullan:
   ```
   cargo build --release
   ```
5. **Kur:**
   ```
   claude-rpc install
   ```
   Bu komut: exe'yi `%LOCALAPPDATA%\claude-rpc\claude-rpc.exe` altına kopyalar (derleme yaparken kilitlenmesin diye), `~/.claude/settings.json`'a hook'ları ekler (önce `settings.json.bak-<zaman>` yedeği alır), `HKCU\...\Run\ClaudeRPC` ile oturum açılışında başlatmayı ekler, daemon'ı başlatır.
   Tekrar çalıştırmak güvenlidir (çift kayıt oluşmaz).
6. Kaldırmak için `claude-rpc uninstall` (hook'lar, Run kaydı, çalışan daemon). Config/state dosyaları `%LOCALAPPDATA%\claude-rpc` içinde kalır, istersen sil.

> **Not (konsol çıktısı):** exe `windows` alt sisteminde olduğundan (hook çalışırken konsol penceresi çakmasın diye) kabuk komutu beklemez. cmd/PowerShell'de çıktıyı düzgün görmek için `claude-rpc status | more` (PowerShell: `| Out-String`) kullan.

### Tepsi simgesi (arayüz)

Daemon çalışırken sağ alttaki bildirim alanında (gizliyse `^` okunun içinde) turuncu bir yıldız simgesi durur. Başlarken "Claude RPC çalışıyor" bildirimi çıkar (`notify_on_start = false` ile kapanır).
Simgeye sağ (veya sol) tıklayınca menü açılır:

- durum satırı: `Bağlı · proje / Aktif`, `Discord'a bağlı değil`, `Duraklatıldı`
- **Presence'ı duraklat / devam ettir** (duraklatınca Discord'daki durum temizlenir)
- **Ayar dosyasını aç** (yoksa örnek config oluşturulup Not Defteri'nde açılır)
- **Veri klasörünü aç**
- **Çıkış** (daemon kapanır, bir sonraki oturum açılışına kadar çalışmaz)

`claude-rpc.exe`'ye çift tıklamak da uygulamayı başlatır; zaten çalışıyorsa bunu söyleyen bir pencere gösterir. Tepsi ikinci bir iş parçacığı (ana iş parçacığı mesaj döngüsünü çalıştırır, Discord döngüsü ayrı thread'dedir); RAM ölçümü tepsiyle birlikte de aynı: ~0,8 MB working set, 268 KB private working set.

### Otomatik güncelleme

Açılıştan ~25 sn sonra ve sonra 6 saatte bir GitHub Releases'te daha yeni bir sürüm var mı diye bakılır. Varsa **"Yeni sürümü var, güncellemek ister misin?"** penceresi çıkar. **Evet**'e basınca yeni `claude-rpc.exe` indirilir, SHA-256 özeti GitHub'daki ile doğrulanır, çalışan exe `.old` olarak yeniden adlandırılıp yenisi yerine konur ve uygulama kendini yeniden başlatır. **Hayır** dersen aynı sürüm için bir daha sormaz (bir sonraki açılışa kadar). Tepsi menüsündeki **Güncellemeleri denetle** her zaman hemen kontrol eder.
İndirme Windows'la gelen `curl.exe` ile yapılır (uygulamaya TLS/HTTP kütüphanesi eklenmedi). Kapatmak için `check_updates = false`. Terminalden: `claude-rpc update` (denetle) / `claude-rpc update --yes` (güncelle), `claude-rpc version`.

### Komutlar

```
claude-rpc install   [--settings <yol>] [--no-registry] [--no-start] [--in-place]
claude-rpc uninstall [--settings <yol>] [--no-registry] [--no-start]
claude-rpc status        # daemon, Discord bağlantısı, son presence, hook oturumları, Claude Desktop
claude-rpc start | stop
claude-rpc daemon        # arka plan süreci (konsolsuz)
claude-rpc hook <olay>   # Claude Code hook'u, stdin'den JSON okur
```

`--settings` / `--no-registry` / `--no-start` / `--in-place` mevcut ayarlarına dokunmadan denemek içindir (aşağıda "Doğrulama").
`CLAUDE_CONFIG_DIR` ayarlıysa `settings.json` ve `projects` oradan okunur.

## Config (`config.toml`)

| Anahtar | Varsayılan | Anlam |
|---|---|---|
| `client_id` | — (zorunlu) | Discord Application ID |
| `idle_minutes` | `5` | son olaydan sonra otomatik "Boşta" |
| `stale_minutes` | `60` | bu süreden sonra oturum bayat, presence temizlenir |
| `active_text` / `idle_text` | `Aktif` / `Bosta` | state metni |
| `chat_details` | `Claude ile sohbet ediyor` | masaüstü sohbet modunda details |
| `large_image`, `small_image_active`, `small_image_idle`, `large_text` | GIF linki, `active`, `idle`, `Claude` | asset anahtarı **veya** herkese açık görsel linki (varsayılan: hareketli GIF, Discord masaüstünde döner; sabit görsel için `claude`) |
| `show_branch` | `true` | branch'i göster |

Config sadece daemon açılırken okunur; değiştirince `claude-rpc stop` + `claude-rpc start`.

## Nasıl çalışıyor

```
Claude Code ──hook──▶ claude-rpc hook <olay> ──atomik yaz──▶ %LOCALAPPDATA%\claude-rpc\state.json
                                                                   │ (2 sn'de bir mtime yoklaması)
                                                    claude-rpc daemon ──▶ \\.\pipe\discord-ipc-N ──▶ Discord
```

Hook olayları: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `Stop`, `SessionEnd`. Format Claude Code'un hook dokümanındaki
`hooks.<Olay>[{matcher?, hooks:[{type:"command", command, timeout}]}]` şeması; stdin JSON'undan `session_id`, `cwd`, `hook_event_name` okunur
(makinedeki resmi eklentilerin `hooks.json` dosyalarıyla doğrulandı). Komut, kabuktan bağımsız çalışsın diye tırnaksız ve `/` ayraçlı yazılır (yolda boşluk varsa tırnaklanır).

**Durum mantığı:** `UserPromptSubmit`/`PreToolUse` → Aktif, `Stop` → Boşta, `SessionEnd` → oturum silinir. Son olaydan `idle_minutes` sonra otomatik Boşta,
`stale_minutes` sonra oturum bayat (temizlenir). Birden çok oturumda en son olay alan gösterilir. Başlangıç zamanı oturumun ilk görüldüğü an; cwd değişirse sıfırlanır.

**Hook güvenilirliği:** her zaman exit 0; panik/hata Claude Code'u engellemez. Eşzamanlı hook'lar `state.lock` ile serileştirilir (en kötü ~200 ms, sonra kilitsiz devam). Ölçülen süre: ~10 ms (süreç başlatma dahil).

**Branch:** `git` çalıştırılmaz; `.git/HEAD` okunur. Üst klasörlere doğru `.git` aranır, `.git` *dosyası* (worktree/submodule) `gitdir:` satırıyla çözülür, detached HEAD'de 7 haneli SHA gösterilir.

**Discord IPC:** `\\.\pipe\discord-ipc-0…9` sırayla `std::fs::OpenOptions` ile açılır. Frame = u32 LE opcode + u32 LE uzunluk + JSON. Handshake (op 0, `{"v":1,"client_id"}`) → READY beklenir → `SET_ACTIVITY` (op 1, nonce + pid).
Ping (3) → Pong (4); Close (2) ve kopma → bağlantı bırakılır, 2 sn → 30 sn üstel backoff ile sessizce yeniden bağlanılır. Okuma `PeekNamedPipe` ile bloklamasızdır.
Activity yalnızca içerik değişince ve en erken 15 sn aralıkla gönderilir (Discord rate limit). Aktif oturum kalmayınca `activity: null` ile temizlenir.

### Kaynak önceliği

1. **Hook oturumu** (Claude Code terminal ve **masaüstü uygulamasındaki Code sekmesi**: ikisi de `~/.claude/settings.json` hook'larını çalıştırır) — tam bilgi.
2. **`~/.claude/projects` fallback'i** (hook yoksa): en son değişen `*.jsonl` oturum dosyasının mtime'ından proje bulunur; < `idle_minutes` → Aktif, < `stale_minutes` → Boşta.
   Klasör adı `cwd`'nin her alfanümerik olmayan karakteri `-` yapılmış hâlidir (`C:\Users\x\my-app` → `C--Users-x-my-app`), geri çevirme kayıplıdır; bu yüzden gerçek dizin ağacı gezilip her girdinin kodlanmış adı eşleştirilir
   (tire, nokta, boşluk içeren adlar doğru çözülür; klasör silinmişse `-`→`\` tahminine düşer). Hook'lar çalışıyorsa bu kaynak, `SessionEnd` sonrası eski oturumu yeniden göstermesin diye yalnızca son hook olayından >30 sn yeni dosya değişikliğinde devreye girer.
3. **Claude Desktop sohbeti** — aşağıya bak.

### Claude Desktop (Claude.exe) sohbet araştırması

Bu makinede bakılanlar ve bulgular:

- **Pencere başlığı:** ana pencerenin başlığı sabit `Claude`; sohbet/proje adı içermiyor → kullanılamaz.
- **`%APPDATA%\Claude`:** MSIX paketi olduğu için burası neredeyse boş (yalnızca bir `.user-data-left…` yer tutucu). Gerçek veri `%LOCALAPPDATA%\Claude-Data\claude_<paket>\` altında: Chromium profili (IndexedDB, Local Storage, Cache …), `claude-code-sessions`, `local-agent-mode-sessions` vb.
  Sohbet içeriği/ismi burada düz metin olarak değil, Chromium'un kendi depolarında; kırılgan ve gizlilik açısından uygunsuz olduğu için **okunmuyor**.
- **Süreç listesi:** ~17 `Claude.exe` süreci (ana + renderer/GPU/utility); komut satırları sohbet/proje bilgisi taşımıyor. Code sekmesinin kullandığı `claude.exe` (CLI) ayrı yolda (`…\Claude-Data\…\claude-code\…`) çalışıyor.
- Ek bulgu: `~/.claude/sessions/<pid>.json` dosyaları (`cwd`, `status`, `updatedAt`) var, ancak belgelenmemiş bir iç biçim; kararlı olmadığı için **kullanılmadı** (hook'lar zaten aynı bilgiyi resmî yoldan veriyor).

**Seçilen yöntem (fallback):** hook oturumu ve proje fallback'i yoksa, `Claude.exe` (CLI olmayan, yol `claude-code` içermeyen) sürecine ait **görünür, başlıklı bir üst-seviye pencere** varsa
`details = "Claude ile sohbet ediyor"`, proje/branch yok. Aktif/Boşta: Claude penceresi ön plandaysa Aktif; `idle_minutes` boyunca ön planda değilse Boşta. Pencere görünür değilse (tepsiye küçültülmüş/kapatılmış) presence gösterilmez.
Neden: süreç var diye değil pencere var diye bakmak, uygulama arka planda açık kalsa bile presence'ın takılı kalmasını önler; ön plan kontrolü ise sohbetin gerçekten kullanıldığına dair elimizdeki tek hafif sinyal.

## Bağımlılıklar

| Crate | Neden |
|---|---|
| `serde_json` (`preserve_order` özelliği) | Hook JSON'unu ve `settings.json`'u güvenle ayrıştırmak; `preserve_order` kullanıcının ayar sırasını ve biçimini bozmamak için. Elle JSON ayrıştırıcısı yazmak hatalı/riskli; maliyeti birkaç yüz KB binary, RAM'e etkisi ihmal edilebilir. `serde` derive kullanılmıyor. |

Başka crate **yok**: `toml` yerine ~40 satırlık anahtar=değer ayrıştırıcı (config düz), Win32 (named pipe peek, süreç listesi, kayıt defteri, mutex, pencere sorgusu) elle `extern "system"` bildirimleriyle (`windows-sys` yok → daha küçük binary).

Release profili: `opt-level="z"`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true`.

## Doğrulama (yapılanlar)

- `cargo test` — 25 test, hepsi geçti: Discord frame encode/decode, **sahte named pipe sunucusuyla** handshake → READY → `SET_ACTIVITY` → ping/pong → close → `activity:null` akışı, `.git/HEAD` (normal, detached, worktree, submodule, git yok), `projects` klasör adı çözümü (tire/nokta/boşluklu yollar), `settings.json` merge/uninstall (mevcut ayarlar, sıra, tekrar çalıştırma, bozuk JSON'a dokunmama), durum makinesi (aktif→boşta→bayat, çok oturum, proje değişimi), atomik state yazımı.
- **Uçtan uca** ([`tools/mock-discord.ps1`](tools/mock-discord.ps1) sahte Discord ile, gerçek daemon + gerçek hook süreçleri): SessionStart/UserPromptSubmit → `my-repo` / `feature/rpc · Aktif`; Stop → `… · Bosta`; SessionEnd → temizleme, sonra masaüstü sohbet fallback'i. Gerçek Discord'a sahte client id ile bağlanmanın reddedilip sonraki pipe'a geçildiği de görüldü.
- **install/uninstall** geçici bir `settings.json` kopyası üzerinde denendi: mevcut `hooks.Stop` kaydı korundu, 5 hook eklendi, ikinci `install` "değişiklik yok" dedi, `uninstall` dosyayı orijinaline birebir döndürdü, yedek alındı. Gerçek `~/.claude/settings.json` ve kayıt defterine dokunulmadı (bu yüzden `install`'ın Run kaydı ve gerçek kopyalama yolu otomatik test edilmedi; elle `claude-rpc install` ile dene).
- Hook süresi ~10 ms (süreç başlatma dahil), her durumda exit 0.

### RAM ölçümü (release, daemon bağlıyken, Windows 11)

70 sn boyunca saniyede bir örneklendi (hook olayları ve 10 sn'lik tarama döngüleri dahil):

| Metrik | Sonuç |
|---|---|
| Private Working Set (perf counter `WorkingSetPrivate`) | ort. ~0,2 MB, **maks. 268 KB** |
| Working Set (paylaşılan DLL sayfaları dahil) | **maks. ~1,1 MB** |
| Private bytes (commit) | ~1,0 MB |
| CPU | ~110 ms / 70 sn (≈ %0,15; çoğu 10 sn'lik süreç/`projects` taraması) |

Hedef (1–2 MB, en fazla 3 MB) tutuyor; bunda `SetProcessWorkingSetSize(-1,-1)` ile dakikada bir yapılan working-set kırpması da payı var (başlangıç ve her ~60 sn). Kırpmasız değer ayrıca ölçülmedi.
Ölçümü tekrarlamak için:
```powershell
$d = Get-Process claude-rpc
(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$($d.Id)").WorkingSetPrivate / 1KB
```

## Sorun giderme

- **Presence görünmüyor:** `claude-rpc status` → "Discord: bağlı" mı? Discord masaüstü uygulaması açık olmalı (tarayıcı sürümü IPC açmaz). Discord *Ayarlar → Etkinlik Gizliliği → "Mevcut etkinliği durum mesajı olarak göster"* açık olmalı.
- **"Application ID bulunamadı":** `config.toml` / `CLAUDE_RPC_CLIENT_ID` ayarla, sonra `claude-rpc start`.
- **Görseller çıkmıyor:** asset anahtarları birebir (`claude`, `active`, `idle`) olmalı; yükledikten sonra birkaç dakika bekle.
- **Hook hiç çalışmıyor:** `~/.claude/settings.json` içinde `claude-rpc ... hook ...` komutlarını gör; Claude Code oturumunu yeniden başlat (hook'lar oturum başında okunur). `claude-rpc status` "Hook oturumları" sayısını gösterir.
- **Hata günlüğü:** `%LOCALAPPDATA%\claude-rpc\daemon.log` (64 KB'ı geçince sıfırlanır), son durum `status.json`.
- **Aktif/Boşta Claude çalışırken Boşta'ya düşüyor:** uzun süre araç çağrısı/olay olmayan işlerde `idle_minutes` dolar; artır.
- **Çıktı görünmüyor:** yukarıdaki "konsol çıktısı" notuna bak.
- **Derleme "Access denied":** çalışan daemon exe'yi kilitler; `claude-rpc stop` (kurulu kopya `%LOCALAPPDATA%` altında olduğu için normalde sorun olmaz).

## Bilinen sınırlar

- Masaüstü sohbet modunda proje/sohbet adı yoktur (veri sunulmuyor, yukarıya bak).
- Hook `PreToolUse` her araç çağrısında çalışır (~10 ms); uzun oturumlarda yük ihmal edilebilir ama sıfır değil.
- Aynı anda birden çok Discord istemcisi (Stable + Canary) varsa ilk bağlanılabilen pipe kullanılır.

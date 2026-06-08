# Архитектура и Контекст для ИИ-агента (Antigravity IDE Integration)

Привет! Если ты читаешь это, значит мы продолжаем работу над проектом **Antigravity**. Это шпаргалка для тебя (ИИ-помощника), чтобы ты моментально вошёл в контекст сложной схемы маршрутизации между IDE, нашим кастомным Менеджером и серверами Google.

## 🏗 Как всё устроено (Новая Архитектура)

Наша цель — заставить официальную IDE-интеграцию (Antigravity 2.0 / Gemini Code Assist) работать через наш локальный Менеджер пула аккаунтов (Antigravity-Manager). 
Ранее использовался костыль в виде Node.js-скрипта (порт 8046). **Сейчас мы перешли на полноценный GUI-клиент (Antigravity-Client).**

**Новая цепочка запросов выглядит так:**
`IDE (в Linux/WSL или Windows) --> Antigravity-Client Прокси (порт 8047) --> Antigravity-Manager (порт 8045 или 8055) --> Google Backend`

### Ключевые компоненты и решения проблем:

1. **Antigravity-Client (Внутренний прокси на 8047):**
   Это полноценное Tauri/Rust приложение. При запуске и нажатии кнопки "Connect" оно:
   * Поднимает свой внутренний HTTP-прокси на порту `8047` (`src-tauri/src/local_proxy.rs`).
   * **Инжектит настройки напрямую в IDE:** Записывает `antigravity.proxyBaseUrl` = `http://127.0.0.1:8047/v1` в базу SQLite (`state.vscdb`), а сгенерированный токен — в системный Keyring (libsecret на Linux или Credential Manager на Windows).
   * Автоматически "убивает" запущенные процессы IDE и стартует их заново с новыми настройками.

2. **Проблема 404 при авторизации (`userinfo` и `tokeninfo`):**
   Прежде чем IDE начнёт слать запросы на генерацию кода, она проверяет валидность токена, обращаясь к эндпоинтам OAuth (например, `/oauth2/v2/userinfo`). Менеджер эти роуты не обрабатывает.
   * **Решение:** Прокси-сервер внутри Клиента (`local_proxy.rs`) перехватывает пути, содержащие `userinfo` или `tokeninfo`, и **не отправляет** их в Менеджер. Вместо этого он локально отвечает `200 OK` с моковым JSON-профилем. IDE считает, что авторизация пройдена успешно.

3. **Сложность с Хардкодом Порта (Наследие порта 8046):**
   Исторически мы *напрямую патчили бинарники IDE*, чтобы отучить их от `cloudcode-pa.googleapis.com`. В бинарники `Antigravity 2.0` (на Linux) был вшит порт `8046`.
   * **Решение:** Если IDE упорно стучится на 8046 вместо 8047, игнорируя SQLite-настройки, значит у нее захардкожен старый порт. Для исправления нужно просканировать и заменить строку `http://127.0.0.1:8046` на `http://127.0.0.1:8047` в файлах:
     * `/usr/share/antigravity/resources/bin/language_server`
     * `/usr/share/antigravity/resources/app.asar` (Архив самого Electron-приложения).

---

## 🛠 Где лежит код и как его запускать

- **Antigravity-Manager (Бэкенд с пулом аккаунтов):** 
  - Папка: `/home/yaaaa/projects/AG-manager/Antigravity-Manager/`
  - Запуск: 
    ```bash
    cd /home/yaaaa/projects/AG-manager/Antigravity-Manager
    source ~/.cargo/env
    npm run tauri dev
    ```

- **Antigravity-Client (Пользовательский интерфейс и локальный прокси):** 
  - Папка: `/home/yaaaa/projects/AG-manager/Antigravity-Client/`
  - Исходник прокси: `src-tauri/src/local_proxy.rs`
  - Исходник инжектора SQLite/Keyring: `src-tauri/src/db.rs`
  - Запуск:
    ```bash
    cd /home/yaaaa/projects/AG-manager/Antigravity-Client
    source ~/.cargo/env
    npm run tauri dev
    ```

---

## 🤖 Промпт для восстановления работы после обновлений

*Если оригинальный репозиторий Клиента обновился, всё сломалось, и вы скачали чистый код, отправьте этот текст ИИ новым сообщением.*

> **PROMPT ДЛЯ ИИ (АВАРИЙНОЕ ВОССТАНОВЛЕНИЕ ПОСЛЕ ОБНОВЛЕНИЯ):**
> 
> Привет! Мы обновили исходный код `Antigravity-Client`, и нам нужно заново настроить проксирование для Antigravity IDE. Мы работаем в WSL (Ubuntu) на Windows.
> 
> **Твоя задача — выполнить 2 шага:**
> 
> **ШАГ 1: Восстановление логики авторизации в прокси-сервере Клиента**
> IDE не умеет работать без OAuth авторизации.
> 1. Найди файл `src-tauri/src/local_proxy.rs` в `Antigravity-Client`.
> 2. Внутри функции маршрутизации (где формируется ответ) добавь перехват URL: если путь содержит `tokeninfo` или `userinfo`, прокси **НЕ ДОЛЖЕН** отправлять запрос в Менеджер (иначе будет 404). Он должен локально возвращать HTTP 200 OK и моковый валидный JSON с данными пользователя (например `{"id": "1", "email": "proxy@antigravity", "verified_email": true}`).
> 
> **ШАГ 2: Проверка хардкода старых портов**
> 1. В старых версиях IDE на Linux мы принудительно захардкодили старый прокси (порт 8046) в бинарники, и IDE может игнорировать настройки из SQLite.
> 2. Проверь наличие файлов `/usr/share/antigravity/resources/bin/language_server` и `/usr/share/antigravity/resources/app.asar`.
> 3. Если внутри них есть строка `http://127.0.0.1:8046`, аккуратно пропатчи ее на `http://127.0.0.1:8047` бинарной заменой (не меняя длину строки!).
>
> **ШАГ 3: Зависания IDE (Deadlock в token_manager.rs)**
> 1. IDE может отправлять 3 параллельных запроса при старте (например `listExperiments`, `loadCodeAssist`, `fetchAvailableModels`).
> 2. Если Менеджер (на порту 8046) зависает и возвращает 503 Service Unavailable (или Timeout), значит проблема во взаимной блокировке (Deadlock) при получении токена.
> 3. В `src-tauri/src/proxy/token_manager.rs` Менеджера используется `SingleFlight` для запросов `fetch_project_id`. Если запрос завис, он вешает все остальные ожидающие потоки. Необходимо убедиться, что `SingleFlight` при ошибке очищает карту: `self.load_code_assist_inflight.remove(&token.account_id);` а не держит мьютексы бесконечно.
> 4. Также проверьте наличие таймаутов на всех `reqwest::Client` (в `project_resolver.rs` и `oauth.rs` используйте `.timeout(Duration::from_secs(2))`), чтобы запросы к Google Sandbox не висели вечно, так как JA3-эмуляция иногда ломает TLS-соединение.
> 
> Действуй!

---

## 🚀 Новые патчи и фиксы (Июнь 2026)

В процессе доработки мы столкнулись с рядом неочевидных проблем, которые были успешно решены. Вот шпаргалка по ним:

### 1. Проблема с CORS во фронтенде (Electron UI)
*Симптом:* Приложение падает с экраном "There was an unexpected issue setting up your account". В логах бэкенда при этом "Auth succeeded".
*Причина:* Chromium внутри Electron делает предзапросы `OPTIONS` (Preflight) к нашему локальному прокси `http://127.0.0.1:8047`, а также ожидает заголовок `Access-Control-Allow-Origin: *` в ответах.
*Решение:* Прокси-сервер (`src-tauri/src/local_proxy.rs`) должен обрабатывать метод `OPTIONS` (возвращать 200 OK) и добавлять заголовок `Access-Control-Allow-Origin: *` ко всем своим ответах на перехваченные роуты (userinfo, tokeninfo, fetchAdminControls, token).

### 2. Прямые запросы к Google API в обход прокси
*Симптом:* Тот же "unexpected issue setting up your account" даже с настроенным CORS.
*Причина:* Помимо `cloudcode.googleapis.com`, фронтенд IDE (`main.js`) и бэкенд (`language_server_linux_x64`) содержат захардкоженные URL `https://www.googleapis.com` и `https://oauth2.googleapis.com`. Они отправляют туда наш фейковый токен `ya29.proxy_managed...` напрямую в обход прокси, Google возвращает 401 Unauthorized, и интерфейс падает.
*Решение:* Бинарный патчинг (с сохранением длины строки) `main.js` и `language_server_linux_x64`.
* `https://www.googleapis.com` (26 байт) -> `http://127.0.0.1:8047/////` (26 байт)
* `https://oauth2.googleapis.com` (29 байт) -> `http://127.0.0.1:8047////////` (29 байт)
(Двойные слеши автоматически игнорируются при парсинге в `local_proxy.rs`).

### 3. Зависание IDE в фоне (Single-Instance Electron)
*Симптом:* Изменения конфига или CORS не применяются, при нажатии "Connect" вылезает та же самая старая ошибка. В папке логов IDE не появляются новые папки запуска.
*Причина:* Antigravity IDE работает как Electron-приложение (Single-instance). Если оно зависло или упало в ошибку, запуск нового процесса `antigravity` просто передает фокус зависшему невидимому окну.
*Решение:* Принудительно завершить процесс: `pkill -f antigravity`.

### 4. Невидимая мышь в клиенте (Баг WSLg Wayland)
*Симптом:* Tauri-приложение запускается из WSL, но мышь над ним не отображается.
*Решение:* Запускать клиент с принудительным использованием X11 бэкенда: `GDK_BACKEND=x11 npm run tauri dev`.

### 5. Блокировка базы SQLite (database is locked)
*Симптом:* Ошибка `database is locked` при попытке инжекта токенов в `state.vscdb`.
*Причина:* IDE оставляет базу в режиме `WAL` (Write-Ahead Logging).
*Решение:* В `db.rs` перед записью выполнять команду `PRAGMA journal_mode=DELETE;`.

### 6. Ошибка макроса Tauri
*Симптом:* Ошибка компиляции `error: expected item` в `lib.rs`.
*Причина:* Макрос `#[tauri::command]` во 2-ой версии Tauri несовместим с модификатором `pub` для асинхронных функций.
*Решение:* Использовать `async fn` без модификатора `pub`.

### 7. Мокирование состояния авторизации в базе
В `state.vscdb` необходимо инжектить не только сам токен (через ключ `oauthTokenInfoSentinelKey`), но и обязательно статус `authState: "loggedIn"` (через ключ `authStateWithContextSentinelKey` внутри того же мульти-стейта). Без этого IDE показывает окно приветствия "Sign In" вместо работы.

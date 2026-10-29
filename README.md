<p align="center"><img src="docs/logo.svg" width="128" alt="sbox-rs"></p>

# sbox-rs

Порт s&box (https://github.com/Facepunch/sbox-public, склонирован в `E:\sbox-ref`) на Rust + wgpu.

Соглашения s&box сохранены: X вперёд, Y влево, Z вверх; углы pitch/yaw/roll в градусах.

![demo](docs/img/demo-scene-2.png)

Сайт: https://lakomoor.github.io/sbox-rs/ · Неофициальный проект, не связан с Facepunch и Valve. Файлы игры в репозиторий не входят. Лицензия: MIT.

`sbox-app --screenshot out.png [--time секунды] [model.vmdl_c]` — рендер кадра в PNG без окна.

## Крейты

| Крейт | Источник в C# | Статус |
|---|---|---|
| `sbox-math` | `Sandbox.System/Math` | Vec/Quat (glam), Angles, Transform, BBox, Ray, Plane |
| `sbox-scene` | `Sandbox.Engine/Scene` | Scene/GameObject/Component, жизненный цикл, иерархия, отложенное удаление, fixed update |
| `sbox-render` | закрытое ядро Source 2 (не в репо) | wgpu: меш, направленный + точечные источники, глубина |
| `sbox-resource` | нативные форматы Source 2 (закрытое ядро) | контейнер `_c`, бинарный KV3 (v2), `vtex_c` (BC1-7, LZ4/ZSTD-мипы, PNG), VBIB (meshopt), меши `vmdl_c` |
| `sbox-app` | `Launcher` | демо-окно (WASD/QE, стрелки) |

## Проверено на реальных данных (game/ из artifacts.sbox.game)

- контейнер: 6878/6878 файлов; KV3: 6999/7014 блоков (15 — внешние blob-блоки, напр. citizen.vmdl_c)
- vtex_c: 2618/2618, размеры мипов совпадают до байта
- vmdl_c: 743/744 мешей (3,05 млн треугольников), нормали единичные
- `cargo run -p sbox-app -- <path>.vmdl_c` — просмотр модели

## Дорожная карта (не сделано)

1. Сериализация сцен/префабов (`*.scene`, `*.prefab` JSON) — `GameObject.Serialize.cs`
2. Файловая система/VPK; vmat_c (материалы), внешние vmesh_c, скелет/анимации (anim_c), vsnd_c, шейдеры `.shader_c`; текстуры в рендерер
3. Материалы и текстуры, тени, PBR
4. Физика (rapier) — замена Havok-обёртки; Trace
5. Ввод (`Input`), звук, UI (Razor/Panel → собственный layout)
6. Сеть (`Networking`), ConVar/ConCmd
7. Скриптинг: C#-игровой код не портируется напрямую — нужен Rust-API или встроенный рантайм

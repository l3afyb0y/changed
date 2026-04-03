# Example Output

This is an illustrative example of the human-readable changelog style `changed`
is aiming for.

## Clean View

```text
# Changes

## 04/03/26

- 9:41am [user/shell] /home/rowen/.config/fish/config.fish: Changed shell config [metadata-only]
- 9:44am [system/boot] /boot/loader/entries/arch.conf: Changed boot config (+1/-1)
- 9:47am [system/build] /etc/makepkg.conf: Changed build config (+2/-1)
```

## Full View

```text
# Changes

## 04/03/26

### 9:44am
Scope: system
/boot/loader/entries/arch.conf
Changed boot config (+1/-1)
(-) options root=UUID=... quiet
(+) options root=UUID=... mitigations=off

### 9:47am
Scope: system
/etc/makepkg.conf
Changed build config (+2/-1)
(-) MAKEFLAGS="-j8"
(+) MAKEFLAGS="-j16"
(+) RUSTFLAGS="-C target-cpu=native"
```

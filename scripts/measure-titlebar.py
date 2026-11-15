# Ширина полосы заголовка в худшем случае (нужен Pillow). Пороги — TITLEBAR_BREAKPOINTS в src/components/layout/nav-tabs.tsx.
import math
from PIL import ImageFont
base = 'node_modules/@fontsource-variable/onest/files/'
import sys
WORDMARK, COMPACT = (int(x) for x in sys.argv[1:3]) if len(sys.argv) > 2 else (1080, 940)

def onest(text, size, weight, cyr):
    f = ImageFont.truetype(base + ('onest-cyrillic-wght-normal.woff2' if cyr else 'onest-latin-wght-normal.woff2'), size)
    f.set_variation_by_axes([weight])
    return f.getlength(text)

def tab(label, act, compact, cyr):
    # Таблетка: отступ 12 + иконка 16 + зазор 6 + подпись + 12; у «Сборок» во время игры — точка 8.
    w = 12 + 16 + 6 + onest(label, 14, 500, cyr) + 12
    if act == 'play':
        w += 6 + 8
    return w

for lang, labels, add, cyr in (("ru", ["Главная", "Сборки", "Каталог", "Скины"], "Добавить аккаунт", True),
                               ("en", ["Home", "Instances", "Discover", "Skins"], "Add account", False)):
    for width in sorted({WORDMARK, WORDMARK - 1, COMPACT, COMPACT - 1, 960}, reverse=True):
        wordmark, compact = width >= WORDMARK, width < COMPACT
        # Отступ 20 + лок-ап высотой 22 + запас 15; уже порога — отступ и искра 22.
        brand = math.ceil(20 + 22 * 747.42 / 116.5 + 15) if wordmark else 20 + 21
        tabs = sum(tab(l, 'play' if i == 1 else None, compact, cyr) for i, l in enumerate(labels)) + 3 * 2
        nav = 8 + 6 + tabs + 6
        # Таблетка аккаунта: 4 + голова 32 + 10 + ник + 10 + шеврон 16 + 10; справа «Загрузки» и «Настройки» по 36.
        acct = 4 + 32 + 10 + (80 if compact else 112) + 10 + 16 + 10
        add_btn = 4 + 28 + 10 + min(onest(add, 14, 600, cyr), 96 if compact else 999) + 10
        right = max(acct, add_btn) + 4 + 36 + 4 + 36 + 8
        total = brand + nav + 0 + right + 5 + 138
        print(f"{lang} {width}: brand {brand} tabs {round(tabs)} account {round(max(acct, add_btn))} total {round(total)} spare {round(width - total)}")

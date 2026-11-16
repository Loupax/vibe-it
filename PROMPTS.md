
## QA11a part 2: border width default medium, keywords, currentColor (BLOCKED, PR open)

> Land the border default/keyword/currentColor change with zero pass->fail.

**What changed:** css.rs `parse_border_width` (thin/medium/thick), style.rs unset width = 3px when a style is set, explicit `currentColor` and `inherit`/`initial` of an unset border colour resolve to the own colour, paint.rs unset colour paints as `color` (`with_current_border_color`). Tests `border_width_defaults_to_medium_and_keywords_parse`, `border_color_current_color_resolves_to_own_color`; `q53_negative_sizes_are_dropped` expectation corrected (dropped width with a style is medium, 43px not 40). WPT +7 new passes, 2 pass->fail (`table-rules-rows-vertical`, `row-group-margin-border-padding`: accidental passes needing QA11a2), so NOT merged.

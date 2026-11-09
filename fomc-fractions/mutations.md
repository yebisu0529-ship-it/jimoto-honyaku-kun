| mutation | file | result | failing tests |
|---|---|---|---|
| drop the ' and ' joiner | src/live_text.rs | caught | rates_in_quarters_and_halves_are_written_as_decimals |
| drop the '-' joiner | src/live_text.rs | caught | rates_in_quarters_and_halves_are_written_as_decimals |
| drop the percent rule | src/live_text.rs | caught | rates_in_quarters_and_halves_are_written_as_decimals |
| percent rule without ' percent' | src/live_text.rs | caught | rates_in_quarters_and_halves_are_written_as_decimals |
| write '' instead of '0' before a bare fraction | src/live_text.rs | caught | rates_in_quarters_and_halves_are_written_as_decimals |
| accept a whole number 0 | src/live_text.rs | caught | rates_in_quarters_and_halves_are_written_as_decimals, text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| accept 3 and 4 digit whole numbers | src/live_text.rs | caught | text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| no left boundary | src/live_text.rs | caught | text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| left boundary without '/' | src/live_text.rs | caught | text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| left boundary without '-' | src/live_text.rs | caught | text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| left boundary without '.' and ',' | src/live_text.rs | caught | text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| no right boundary | src/live_text.rs | caught | text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| right boundary lets '/' through | src/live_text.rs | caught | text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| '.' or ',' then a digit ends the fraction | src/live_text.rs | caught | text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| swap the 1/4 and 3/4 decimals | src/live_text.rs | caught | rates_in_quarters_and_halves_are_written_as_decimals |
| re-join the output on whitespace | src/live_text.rs | caught | rates_in_quarters_and_halves_are_written_as_decimals, text_that_is_not_a_rate_in_quarters_or_halves_comes_back_byte_for_byte |
| fractions before fillers | src/document.rs | caught | fillers_are_left_out_before_the_fractions_are_read |
| no fractions for spoken text (main) | src/document.rs | caught | a_live_caption_reaches_the_translation_server_with_its_rate_as_a_decimal, a_rate_in_fractions_stays_in_the_source_but_is_a_decimal_in_what_the_translator_reads, fillers_are_left_out_before_the_fractions_are_read |
| no fractions in the previous source | src/document.rs | caught | a_rate_in_fractions_stays_in_the_source_but_is_a_decimal_in_what_the_translator_reads |
| no fractions for screen text | src/document.rs | caught | screen_text_keeps_its_fillers_but_its_fractions_become_decimals |
| screen text loses its fillers too | src/document.rs | caught | fillers_stay_in_screen_text_and_in_text_that_may_not_be_english, screen_text_keeps_its_fillers_but_its_fractions_become_decimals |
| fractions in text that is not English | src/document.rs | caught | fractions_stay_in_text_that_is_not_english |

22 of 22 mutations caught
restored tree: lib live_text tests ['36'] failed []; GUI document tests ['31'] failed []

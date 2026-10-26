/* SPDX-License-Identifier: MIT
 * Bounded regression for CVE-2026-107676. Exercise the public serializer and
 * parser, without reading real process memory or supplying hostile media.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "libavutil/hdr_dynamic_metadata.h"
#include "libavutil/mem.h"

static int check_case(int windows, unsigned flags)
{
    AVDynamicHDRPlus input = {0}, decoded = {0};
    uint8_t first[1024], second[1024], *a = first, *b = second, *allocated = NULL;
    size_t required = 0, a_size = sizeof(first), b_size = sizeof(second), allocated_size = 0;
    const char *failure = NULL;

    input.application_version = 1;
    input.num_windows = windows;
    input.targeted_system_display_maximum_luminance = (AVRational){1000, 1};
    for (int w = 0; w < windows; w++) {
        AVHDRPlusColorTransformParams *p = &input.params[w];
        p->window_upper_left_corner_x = p->window_upper_left_corner_y = (AVRational){0, 1};
        p->window_lower_right_corner_x = p->window_lower_right_corner_y = (AVRational){1, 1};
        for (int c = 0; c < 3; c++)
            p->maxscl[c] = (AVRational){0, 1};
        p->average_maxrgb = p->fraction_bright_pixels = (AVRational){0, 1};
        p->tone_mapping_flag = (flags >> (2 * w)) & 1;
        p->color_saturation_mapping_flag = (flags >> (2 * w + 1)) & 1;
        p->knee_point_x = (AVRational){1234, 4095};
        p->knee_point_y = (AVRational){2345, 4095};
        p->color_saturation_weight = (AVRational){w + 1, 8};
    }
    memset(first, 0xa5, sizeof(first));
    memset(second, 0x5a, sizeof(second));
    if (av_dynamic_hdr_plus_to_t35(&input, NULL, &required) < 0 ||
        !required || required >= sizeof(first)) {
        failure = "invalid size query";
    } else if (av_dynamic_hdr_plus_to_t35(&input, &a, &a_size) < 0 ||
               av_dynamic_hdr_plus_to_t35(&input, &b, &b_size) < 0 ||
               a != first || b != second || a_size != required || b_size != required) {
        failure = "caller-owned output failed";
    } else if (memcmp(first, second, required)) {
        failure = "output depends on previous buffer contents";
    } else {
        for (size_t i = required; i < sizeof(first); i++) {
            if (first[i] != 0xa5 || second[i] != 0x5a) {
                failure = "wrote beyond reported payload size";
                break;
            }
        }
    }
    if (!failure && (av_dynamic_hdr_plus_to_t35(&input, &allocated, &allocated_size) < 0 ||
                     allocated_size != required || memcmp(first, allocated, required)))
        failure = "allocated and caller-owned output differ";
    if (!failure && (av_dynamic_hdr_plus_from_t35(&decoded, first, required) < 0 ||
                     decoded.num_windows != windows || decoded.application_version != 1))
        failure = "payload does not round-trip";
    for (int w = 0; !failure && w < windows; w++) {
        const AVHDRPlusColorTransformParams *p = &input.params[w], *q = &decoded.params[w];
        if (q->tone_mapping_flag != p->tone_mapping_flag ||
            q->color_saturation_mapping_flag != p->color_saturation_mapping_flag ||
            (p->color_saturation_mapping_flag &&
             q->color_saturation_weight.num * 8 != (w + 1) * q->color_saturation_weight.den) ||
            (p->tone_mapping_flag &&
             (q->knee_point_x.num != 1234 || q->knee_point_y.num != 2345)))
            failure = "tone-mapping or saturation metadata changed";
    }
    av_free(allocated);
    if (failure)
        fprintf(stderr, "windows=%d flags=%u: %s\n", windows, flags, failure);
    return failure != NULL;
}

int main(void)
{
    int cases = 0, failures = 0;
    for (int windows = 1; windows <= 3; windows++) {
        for (unsigned flags = 0; flags < (1u << (2 * windows)); flags++) {
            cases++;
            failures += check_case(windows, flags);
        }
    }
    printf("HDR10+ metadata: %d cases, %d failures\n", cases, failures);
    return failures ? 1 : 0;
}

#include <stdint.h>
#include <stddef.h>
int32_t teto_phone_spike(const uint8_t *assets, size_t assets_len,
                       const uint8_t *report, size_t report_len);
int32_t teto_phone_cache_probe(const uint8_t *assets, size_t assets_len,
                               const uint8_t *report, size_t report_len);
int32_t teto_phone_set_host(const uint8_t *device, size_t device_len,
                            const uint8_t *os, size_t os_len);
void teto_phone_cancel(void);

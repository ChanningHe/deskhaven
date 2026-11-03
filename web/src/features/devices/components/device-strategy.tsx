import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'

import type { EffectiveStrategyView, StrategyLayerView } from '@/api/generated/model'

function layerLabel(layer: StrategyLayerView, t: TFunction) {
  return t('devices.strategy.source', {
    scope: t(`devices.strategy.scope.${layer.scope}`),
    name: layer.strategy_name,
  })
}

/** The strategy the device ends up with and the layer each option comes from (05 §4.3). */
export function DeviceStrategy({ strategy }: { strategy: EffectiveStrategyView }) {
  const { t } = useTranslation()
  return (
    <section aria-label={t('devices.strategy.title')} className="grid gap-2">
      <h3 className="text-sm font-medium">{t('devices.strategy.title')}</h3>
      {strategy.layers.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t('devices.strategy.none')}</p>
      ) : (
        <>
          <p className="text-xs text-muted-foreground">{t('devices.strategy.layers')}</p>
          <ol className="list-decimal pl-5 text-sm">
            {strategy.layers.map((layer) => (
              <li key={`${layer.scope}-${String(layer.target_id)}`}>{layerLabel(layer, t)}</li>
            ))}
          </ol>
          <table className="w-full text-left text-xs">
            <caption className="sr-only">{t('devices.strategy.options')}</caption>
            <tbody>
              {strategy.options.map((option) => (
                <tr key={option.key} className="border-t align-top">
                  <th scope="row" className="py-1 pr-2 font-mono font-normal">
                    {option.key}
                  </th>
                  <td className="py-1 pr-2 break-all">
                    {option.value === '' ? (
                      <span className="text-muted-foreground">{t('devices.strategy.empty')}</span>
                    ) : (
                      option.value
                    )}
                  </td>
                  <td className="py-1 text-muted-foreground">{layerLabel(option.source, t)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </section>
  )
}

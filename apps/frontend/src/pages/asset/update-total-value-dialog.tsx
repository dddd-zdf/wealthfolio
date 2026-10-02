import { zodResolver } from "@hookform/resolvers/zod";
import { DatePickerInput, MoneyInput, useAmountFormatting } from "@wealthfolio/ui";
import { Button } from "@wealthfolio/ui/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@wealthfolio/ui/components/ui/dialog";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@wealthfolio/ui/components/ui/form";
import { format } from "date-fns";
import { useEffect } from "react";
import { useForm, useWatch, type Resolver } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { z } from "zod";

const schema = z.object({
  value: z.coerce.number().positive(),
  date: z.date(),
});
type FormValues = z.infer<typeof schema>;

interface UpdateTotalValueDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currency: string;
  /** Units held at the end of a day (yyyy-MM-dd). */
  unitsOn: (day: string) => number;
  onSave: (date: Date, price: number) => void;
}

/** Enter a manually priced holding's total value; saves the matching unit price. */
export function UpdateTotalValueDialog({
  open,
  onOpenChange,
  currency,
  unitsOn,
  onSave,
}: UpdateTotalValueDialogProps) {
  const { t } = useTranslation();
  const formatting = useAmountFormatting();
  const form = useForm<FormValues>({
    resolver: zodResolver(schema) as Resolver<FormValues>,
    defaultValues: { date: new Date() },
  });

  useEffect(() => {
    if (open) form.reset({ date: new Date() });
  }, [open, form]);

  const [value, date] = useWatch({ control: form.control, name: ["value", "date"] });
  const units = date ? unitsOn(format(date, "yyyy-MM-dd")) : 0;
  const price = units > 0 && Number(value) > 0 ? Number(value) / units : null;

  const onSubmit = (data: FormValues) => {
    if (price == null) return;
    onSave(data.date, price);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[420px]">
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-6">
            <DialogHeader>
              <DialogTitle>{t("asset:updateValuation.update_value")}</DialogTitle>
              <DialogDescription>{t("asset:updateValuation.total_description")}</DialogDescription>
            </DialogHeader>
            <div className="space-y-4">
              <FormField
                control={form.control}
                name="value"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t("asset:updateValuation.new_value")}</FormLabel>
                    <FormControl>
                      <MoneyInput
                        ref={field.ref}
                        name={field.name}
                        value={field.value}
                        onValueChange={field.onChange}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="date"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t("asset:updateValuation.as_of_date")}</FormLabel>
                    <FormControl>
                      <DatePickerInput value={field.value} onChange={field.onChange} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              {price != null && (
                <p className="text-muted-foreground text-sm">
                  {t("asset:updateValuation.units_at_price", {
                    units: units.toLocaleString(undefined, { maximumFractionDigits: 6 }),
                    price: formatting.formatPrice(price, currency, false),
                  })}
                </p>
              )}
            </div>
            <DialogFooter className="gap-2">
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                {t("common:cancel")}
              </Button>
              <Button type="submit" disabled={price == null}>
                {t("asset:updateValuation.update_value")}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

let
    Source = #table(type table [Group = text, Score = number, Eligible = logical, Rank = number], {}),
    PowerBICliGroupedRankInput = Table.RemoveColumns(Source, {"Rank"}),
    PowerBICliGroupedRankSorted = Table.Sort(PowerBICliGroupedRankInput, {{"Group", Order.Ascending}, {"Score", Order.Descending}}),
    PowerBICliGroupedRankGrouped = Table.Group(
        PowerBICliGroupedRankSorted,
        {"Group"},
        {
            {
                "__PowerBICliGroupedRankRows",
                (PowerBICliGroupedRankRows as table) as table =>
                    let
                        PowerBICliGroupedRankSource = Table.Buffer(PowerBICliGroupedRankRows),
                        PowerBICliGroupedRankEligible = Table.SelectRows(PowerBICliGroupedRankSource, each [Eligible] = true),
                        PowerBICliGroupedRankIneligible = Table.SelectRows(PowerBICliGroupedRankSource, each not ([Eligible] = true)),
                        PowerBICliGroupedRankIndexed = Table.AddIndexColumn(PowerBICliGroupedRankEligible, "Rank", 1, 1, Int64.Type),
                        PowerBICliGroupedRankZeroed = Table.AddColumn(PowerBICliGroupedRankIneligible, "Rank", each 0, Int64.Type)
                    in
                        Table.Combine({PowerBICliGroupedRankIndexed, PowerBICliGroupedRankZeroed}),
                type table
            }
        },
        GroupKind.Local
    ),
    PowerBICliGroupedRankCombined = Table.Combine(Table.Column(PowerBICliGroupedRankGrouped, "__PowerBICliGroupedRankRows")),
    PowerBICliGroupedRankTyped = Table.TransformColumnTypes(PowerBICliGroupedRankCombined, {{"Rank", Int64.Type}})
in
    PowerBICliGroupedRankTyped

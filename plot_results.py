import pandas as pd
import seaborn as sns
import matplotlib.pyplot as plt
import re
import numpy as np
from matplotlib.ticker import LogLocator

sns.set_theme("paper", style="whitegrid", palette="Set2")

# In a real application, you would load data from a CSV file
df = pd.read_csv('results/results.csv')

# Process the bench_name to extract version and parallelism
def extract_version_and_parallelism(bench_name):
    if bench_name.startswith('rust-'):
        match = re.search(r'rust-(\d+)', bench_name)
        if match:
            return 'Renoir', int(match.group(1))
        return 'Renoir', 1
    
    if bench_name == "base":
        return "Baseline", 1
    if bench_name == "opt":
        return "Convolution", 1
    return bench_name.lower(), 1

def parse_latency(lat):
    match = re.search(r'^(?:(\d+)s)?(?:(\d+)ms)?(?:(\d+)µs)?(?:(\d+)ns)?$', lat)
    t = 0.0
    t += float(match.group(1) or 0.0)
    t += 1e-3 * float(match.group(2) or 0.0)
    t += 1e-6 * float(match.group(3) or 0.0)
    t += 1e-9 * float(match.group(4) or 0.0)
    return t

# Apply the extraction function
df['version'] = None
df['parallelism'] = None
df['experiment'] = None
df['latency_mean'] = df["latency_mean"].apply(parse_latency)

for idx, row in df.iterrows():
    version, parallelism = extract_version_and_parallelism(row['bench_name'])
    df.at[idx, 'version'] = version
    df.at[idx, 'parallelism'] = parallelism
    df.at[idx, 'experiment'] = version + f"-{parallelism:02}" if parallelism else ""

df.sort_values(by=["experiment", "parallelism", "bench_id"], kind="stable", inplace=True)

# Function to create the catplot for both throughput and latency
def create_catplot(data, y_column, y_label):
    # Create a catplot with versions as columns
    xranges = data.groupby("version")["parallelism"].nunique()
    print(xranges)

    g = sns.catplot(
        data=data,
        x='parallelism',
        y=y_column,
        hue='version',
        markers=["o","s","x"],
        # col='version',
        kind='point',
        height=2,
        aspect=1.6,
        markersize=5,
        palette=["black", "dimgrey", "grey"],
        hue_order=["Baseline","Convolution","Renoir"],
        linewidth=1.5,
        legend="brief",
        sharex=False,
        legend_out=False,
        # facet_kws=dict(gridspec_kws=dict(width_ratios=xranges))
    )

    g.set_titles("{col_name}")
    g.set_xlabels("")
    g.set_ylabels("Throughput(batches/s)")
    ax = plt.gca()

    # Remove background from legend and make it smaller
    legend = ax.get_legend()
    if legend:
        legend.set_title("")
        legend.set_frame_on(True)
        legend.set_bbox_to_anchor((0.5, 0.5))
        for text in legend.get_texts():
            text.set_fontsize(8)
    # sns.move_legend(ax, "bottom right")
    ax.yaxis.set_minor_locator(LogLocator(base=10.0, subs=np.arange(1, 10), numticks=100))
    ax.yaxis.grid(True, which="minor", color="whitesmoke")
    ax.yaxis.set_minor_formatter(plt.NullFormatter())
    plt.ylim((0.2, 2000))

    plt.xlabel("Level of Parallelism")

    plt.savefig(f"results/renoir-gc-{y_column}.pdf", bbox_inches="tight")
    plt.semilogy()
    plt.savefig(f"results/renoir-gc-{y_column}-logy.pdf", bbox_inches="tight")

    # axes = g.axes.flatten()
    
    # # For each version panel, add custom elements
    # for i, (version, group) in enumerate(data.groupby('version')):
    #     if version == 'Rust':
    #         # Sort by parallelism for the line
    #         group = group.sort_values('parallelism')
            
    #         # Plot the line connecting the Rust points
    #         axes[i].plot(
    #             group['parallelism_str'],
    #             group[y_column],
    #             color='blue',
    #             linestyle='-',
    #             alpha=0.7
    #         )
    
    # # Set titles and labels
    # for i, ax in enumerate(axes):
    #     version = data['version'].unique()[i]
    #     ax.set_title(f"{version}")
    #     ax.set_xlabel('Parallelism' if version == 'Rust' else '')
    #     ax.set_ylabel(y_label)
    
    return g

# Create the throughput plot
throughput_fig = create_catplot(
    df, 
    'throughput',
    # 'Throughput Comparison Across Versions', 
    'Throughput (ops/sec)'
)
plt.show()

# Create the latency plot
latency_fig = create_catplot(
    df, 
    'latency_mean', 
    # 'Mean Latency Comparison Across Versions',
    'Mean Latency (ms)'
)

# Adjust to make plot more visually appealing
plt.show()

# For real application, you would use this code to load data from a CSV file:
# """
# def load_and_process_data(csv_path):
#     df = pd.read_csv(csv_path)
    
#     # Process the bench_name to extract version and parallelism
#     def extract_version_and_parallelism(bench_name):
#         if bench_name.startswith('rust-'):
#             match = re.search(r'rust-(\d+)', bench_name)
#             if match:
#                 return 'Rust', int(match.group(1))
#             return 'Rust', None
#         return bench_name.capitalize(), None
    
#     # Apply the extraction function
#     df['version'] = None
#     df['parallelism'] = None
#     for idx, row in df.iterrows():
#         version, parallelism = extract_version_and_parallelism(row['bench_name'])
#         df.at[idx, 'version'] = version
#         df.at[idx, 'parallelism'] = parallelism
    
#     # Convert parallelism to string for display, only for those with values
#     df['parallelism_str'] = df['parallelism'].apply(lambda x: f"{x}" if pd.notnull(x) else "")
    
#     return df

# # Usage:
# # df = load_and_process_data('your_benchmark_data.csv')
# """


# g = sns.catplot(
#     data=df,
#     x='parallelism',
#     y="throughput",
#     hue="version", # Color by original version, this connects points for scaling variant
#     kind='point',
#     height=6,
#     aspect=1.5, # Adjust aspect ratio for more x-categories
#     errorbar="ci", # or "sd"
#     capsize=.1,
#     dodge=False, # Points for the same hue are already separated by x_category
#     # order=x_order, # Not needed if x_category is pd.Categorical with order
#     palette='viridis' # Example palette
# )

# g.set_xticklabels(rotation=45, ha='right')
# g.set_axis_labels("Variant / Parallelism Level", "Throughput")
# g.fig.suptitle("Throughput Comparison with Scaling Detail", y=1.03)
# plt.tight_layout()
# plt.show()

